mod theme;

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState};
use ratatui::Frame;
use ratatui::Terminal;

use crate::discover::WorkerGroup;
use crate::logs;
use crate::orchestrator::Orchestrator;
use crate::status::WorkerView;
use theme::{
    confirm_prompt_style, engine_style, engine_url_style, footer_style, group_header_style,
    header_accent_style, hint_style, log_title_style, muted_cell_style, non_spawnable_style,
    overlay_bg_style, process_style, selection_row_style, status_style,
};

enum UiMode {
    Dashboard,
    ConfirmRestart(String),
    Busy(String),
}

/// Latest dashboard data produced by the background poller. Carries the
/// engine-query error (if any) so the UI can show "engine unreachable" rather
/// than silently rendering every worker as disconnected.
#[derive(Clone, Default)]
struct DashboardState {
    views: Vec<WorkerView>,
    engine_error: Option<String>,
}

enum DisplayRowKind {
    Header(WorkerGroup),
    Worker(usize),
}

struct DisplayRow {
    kind: DisplayRowKind,
}

pub async fn run(orchestrator: Arc<Orchestrator>) -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;

    let result: Result<()> = async {
        let stdout = io::stdout();
        let backend = ratatui::backend::CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Seed the dashboard, then poll the engine on a background task so a
        // slow or unreachable engine query can't freeze keyboard input.
        let (initial_views, initial_err) = orchestrator.dashboard_snapshot().await;
        let (state_tx, state_rx) = tokio::sync::watch::channel(DashboardState {
            views: initial_views,
            engine_error: initial_err,
        });
        let poll_interval = Duration::from_millis(orchestrator.config.poll_interval_ms);
        let poller = {
            let orchestrator = orchestrator.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(poll_interval).await;
                    let (views, engine_error) = orchestrator.dashboard_snapshot().await;
                    if state_tx.send(DashboardState { views, engine_error }).is_err() {
                        break; // UI gone
                    }
                }
            })
        };

        // Number of worker actions (start/stop/restart) still running. The Busy
        // overlay clears only when this hits zero, and a new action can't be
        // fired while it's non-zero — preventing interleaved start/stop on the
        // same worker from a fast keypress.
        let in_flight = Arc::new(AtomicUsize::new(0));

        let mut state = state_rx.borrow().clone();
        let mut display_rows = build_display_rows(&state.views);
        let mut table_state = TableState::default();
        table_state.select(Some(first_worker_row(&display_rows).unwrap_or(0)));

        let mut mode = UiMode::Dashboard;
        let mut last_busy_tick = Instant::now();
        let mut running = true;

        let color_enabled = orchestrator.config.color_mode.enabled_for_tui();

        while running {
            state = state_rx.borrow().clone();
            display_rows = build_display_rows(&state.views);
            terminal.draw(|f| {
                draw_ui(
                    f,
                    &orchestrator.config.engine_url,
                    &state.views,
                    state.engine_error.as_deref(),
                    &display_rows,
                    &mut table_state,
                    &mode,
                    color_enabled,
                );
            })?;

            // Clear the Busy overlay once actions finish. Tick on the poll
            // cadence so brief informational messages stay readable, but never
            // clear while an action is still running.
            if last_busy_tick.elapsed() >= poll_interval {
                last_busy_tick = Instant::now();
                if matches!(mode, UiMode::Busy(_)) && in_flight.load(Ordering::SeqCst) == 0 {
                    mode = UiMode::Dashboard;
                }
            }

            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    match &mode {
                        UiMode::ConfirmRestart(worker) => {
                            let worker = worker.clone();
                            match key.code {
                                KeyCode::Char('y') | KeyCode::Enter => {
                                    spawn_restart(
                                        orchestrator.clone(),
                                        in_flight.clone(),
                                        worker.clone(),
                                    );
                                    mode = UiMode::Busy(format!("restarting {worker} + dependents…"));
                                }
                                KeyCode::Char('n') | KeyCode::Esc => {
                                    mode = UiMode::Dashboard;
                                }
                                _ => {}
                            }
                        }
                        UiMode::Busy(_) => {
                            // Esc dismisses an informational message, but not an
                            // action that's still running.
                            if key.code == KeyCode::Esc
                                && in_flight.load(Ordering::SeqCst) == 0
                            {
                                mode = UiMode::Dashboard;
                            }
                        }
                        UiMode::Dashboard => {
                            running = handle_dashboard_key(
                                orchestrator.clone(),
                                in_flight.clone(),
                                key,
                                &mut table_state,
                                &display_rows,
                                &state.views,
                                &mut mode,
                            )
                            .await?;
                        }
                    }
                }
            }
        }

        poller.abort();
        Ok(())
    }
    .await;

    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    result
}

#[allow(clippy::too_many_arguments)]
async fn handle_dashboard_key(
    orchestrator: Arc<Orchestrator>,
    in_flight: Arc<AtomicUsize>,
    key: KeyEvent,
    table_state: &mut TableState,
    display_rows: &[DisplayRow],
    views: &[WorkerView],
    mode: &mut UiMode,
) -> Result<bool> {
    let worker_name = selected_worker(display_rows, views, table_state.selected().unwrap_or(0));

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(false),
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(i) = move_selection(display_rows, table_state.selected().unwrap_or(0), false)
            {
                table_state.select(Some(i));
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(i) = move_selection(display_rows, table_state.selected().unwrap_or(0), true) {
                table_state.select(Some(i));
            }
        }
        KeyCode::Char('r') => {
            if let Some(name) = worker_name {
                if views.iter().any(|v| v.name == name && v.spawnable) {
                    *mode = UiMode::ConfirmRestart(name);
                } else {
                    *mode = UiMode::Busy(
                        "worker not startable from workers-dev (use iii worker add)".into(),
                    );
                }
            }
        }
        KeyCode::Char('s') => {
            if let Some(name) = worker_name {
                if views.iter().any(|v| v.name == name && v.spawnable) {
                    spawn_start(orchestrator.clone(), in_flight.clone(), vec![name.clone()]);
                    *mode = UiMode::Busy(format!("starting {name}…"));
                } else {
                    *mode = UiMode::Busy(
                        "worker not startable from workers-dev (use iii worker add)".into(),
                    );
                }
            }
        }
        KeyCode::Char('x') => {
            if let Some(name) = worker_name {
                spawn_stop(orchestrator.clone(), in_flight.clone(), vec![name.clone()]);
                *mode = UiMode::Busy(format!("stopping {name}…"));
            }
        }
        KeyCode::Char('l') => {
            if let Some(name) = worker_name {
                disable_raw_mode()?;
                execute!(io::stdout(), LeaveAlternateScreen)?;
                println!("Following logs for {name} (Ctrl+C to return)…\n");
                let _ = crate::commands::run_logs(orchestrator.clone(), name, true, 20).await;
                enable_raw_mode()?;
                execute!(io::stdout(), EnterAlternateScreen)?;
            }
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            spawn_start_harness_stack(orchestrator.clone(), in_flight.clone());
            *mode = UiMode::Busy("starting harness stack…".to_string());
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            spawn_start(
                orchestrator.clone(),
                in_flight.clone(),
                orchestrator.config.workers.clone(),
            );
            *mode = UiMode::Busy("starting all managed workers…".to_string());
        }
        _ => {}
    }
    Ok(true)
}

/// Run a worker action on a detached task, counting it as in-flight for the
/// whole duration so the UI keeps the Busy overlay up and refuses overlapping
/// actions until it finishes.
fn spawn_action<F>(in_flight: Arc<AtomicUsize>, fut: F)
where
    F: std::future::Future<Output = Result<()>> + Send + 'static,
{
    in_flight.fetch_add(1, Ordering::SeqCst);
    tokio::spawn(async move {
        if let Err(err) = fut.await {
            eprintln!("workers-dev: {err:#}");
        }
        in_flight.fetch_sub(1, Ordering::SeqCst);
    });
}

fn spawn_start(orchestrator: Arc<Orchestrator>, in_flight: Arc<AtomicUsize>, names: Vec<String>) {
    spawn_action(in_flight, async move {
        orchestrator.start_workers(&names, false).await
    });
}

fn spawn_start_harness_stack(orchestrator: Arc<Orchestrator>, in_flight: Arc<AtomicUsize>) {
    spawn_action(in_flight, async move {
        orchestrator.start_harness_stack(false).await
    });
}

fn spawn_stop(orchestrator: Arc<Orchestrator>, in_flight: Arc<AtomicUsize>, names: Vec<String>) {
    spawn_action(in_flight, async move { orchestrator.stop_workers(&names).await });
}

fn spawn_restart(orchestrator: Arc<Orchestrator>, in_flight: Arc<AtomicUsize>, worker: String) {
    spawn_action(in_flight, async move {
        orchestrator.restart_worker(&worker).await
    });
}

fn build_display_rows(views: &[WorkerView]) -> Vec<DisplayRow> {
    let mut rows = Vec::new();
    let mut last_group = None;
    for (idx, view) in views.iter().enumerate() {
        if last_group != Some(view.group) {
            rows.push(DisplayRow {
                kind: DisplayRowKind::Header(view.group),
            });
            last_group = Some(view.group);
        }
        rows.push(DisplayRow {
            kind: DisplayRowKind::Worker(idx),
        });
    }
    rows
}

fn first_worker_row(display_rows: &[DisplayRow]) -> Option<usize> {
    display_rows.iter().position(|row| matches!(row.kind, DisplayRowKind::Worker(_)))
}

fn move_selection(display_rows: &[DisplayRow], current: usize, down: bool) -> Option<usize> {
    let mut i = current;
    loop {
        if down {
            if i + 1 >= display_rows.len() {
                return None;
            }
            i += 1;
        } else if i == 0 {
            return None;
        } else {
            i -= 1;
        }
        if matches!(display_rows[i].kind, DisplayRowKind::Worker(_)) {
            return Some(i);
        }
    }
}

fn selected_worker(
    display_rows: &[DisplayRow],
    views: &[WorkerView],
    row: usize,
) -> Option<String> {
    match display_rows.get(row)?.kind {
        DisplayRowKind::Worker(idx) => views.get(idx).map(|v| v.name.clone()),
        DisplayRowKind::Header(_) => None,
    }
}

fn selected_worker_view<'a>(
    display_rows: &[DisplayRow],
    views: &'a [WorkerView],
    row: usize,
) -> Option<&'a WorkerView> {
    match display_rows.get(row)?.kind {
        DisplayRowKind::Worker(idx) => views.get(idx),
        DisplayRowKind::Header(_) => None,
    }
}

fn styled_if(enabled: bool, style: Style) -> Style {
    if enabled {
        style
    } else {
        Style::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_ui(
    f: &mut Frame,
    engine_url: &str,
    views: &[WorkerView],
    engine_error: Option<&str>,
    display_rows: &[DisplayRow],
    table_state: &mut TableState,
    mode: &UiMode,
    color_enabled: bool,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(22),
            Constraint::Length(1),
        ])
        .split(f.area());

    let mut header_spans = vec![
        Span::styled(
            "workers-dev",
            styled_if(color_enabled, header_accent_style()).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  engine "),
        Span::styled(engine_url, styled_if(color_enabled, engine_url_style())),
    ];
    if engine_error.is_some() {
        header_spans.push(Span::styled(
            "  ⚠ unreachable",
            styled_if(color_enabled, Style::default().fg(Color::Red)).add_modifier(Modifier::BOLD),
        ));
    }
    let header = Paragraph::new(Line::from(header_spans))
        .block(Block::default().borders(Borders::ALL).title(" workers-dev "));
    f.render_widget(header, chunks[0]);

    let rows: Vec<Row> = display_rows
        .iter()
        .map(|row| match row.kind {
            DisplayRowKind::Header(group) => Row::new(vec![Cell::from(format!(
                "── {} ──",
                group.label()
            ))
            .style(styled_if(color_enabled, group_header_style(group)))]),
            DisplayRowKind::Worker(idx) => {
                let v = &views[idx];
                let icon = status_icon(&v.display_status);
                let name_cell = if v.spawnable {
                    Cell::from(Span::styled(
                        format!("{icon} {}", v.name),
                        styled_if(color_enabled, status_style(&v.display_status)),
                    ))
                } else {
                    Cell::from(Line::from(vec![
                        Span::styled(
                            format!("{icon} {} ", v.name),
                            styled_if(color_enabled, status_style(&v.display_status)),
                        ),
                        Span::styled(
                            "(iii worker add)",
                            styled_if(color_enabled, non_spawnable_style()),
                        ),
                    ]))
                };
                let pid = v
                    .local_pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "—".to_string());
                Row::new(vec![
                    name_cell,
                    Cell::from(Span::styled(
                        v.process_status.clone(),
                        styled_if(color_enabled, process_style(&v.process_status)),
                    )),
                    Cell::from(Span::styled(
                        v.engine_status.clone(),
                        styled_if(color_enabled, engine_style(&v.engine_status)),
                    )),
                    Cell::from(Span::styled(
                        pid.clone(),
                        styled_if(color_enabled, muted_cell_style_for(&pid)),
                    )),
                    Cell::from(Span::styled(
                        v.uptime.clone(),
                        styled_if(color_enabled, muted_cell_style_for(&v.uptime)),
                    )),
                ])
            }
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(38),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(12),
            Constraint::Percentage(22),
        ],
    )
    .header(
        Row::new(vec!["Worker", "Process", "Engine", "PID", "Uptime"]).style(
            Style::default().add_modifier(Modifier::BOLD),
        ),
    )
    .block(Block::default().borders(Borders::ALL).title(" Workers "))
    .row_highlight_style(styled_if(color_enabled, selection_row_style()));
    f.render_stateful_widget(table, chunks[1], table_state);

    let selected_row = table_state.selected().unwrap_or(0);
    render_log_pane(
        f,
        chunks[2],
        selected_worker_view(display_rows, views, selected_row),
        color_enabled,
    );

    let (help, overlay) = match mode {
        UiMode::Dashboard => (
            " ↑↓ select  s start  x stop  r restart  l logs  Ctrl+u harness stack  Ctrl+a all  q quit ",
            None,
        ),
        UiMode::ConfirmRestart(name) => (
            " ↑↓ select  s start  x stop  r restart  l logs  Ctrl+u harness stack  Ctrl+a all  q quit ",
            Some(format!("Restart {name} and dependents?  y/Enter yes  n/Esc no")),
        ),
        UiMode::Busy(msg) => (msg.as_str(), None),
    };
    let footer = Paragraph::new(help).style(styled_if(color_enabled, footer_style()));
    f.render_widget(footer, chunks[3]);

    if let Some(text) = overlay {
        draw_overlay(f, chunks[1], &text, color_enabled);
    }
}

fn muted_cell_style_for(value: &str) -> Style {
    if value == "—" {
        muted_cell_style()
    } else {
        Style::default()
    }
}

fn render_log_pane(f: &mut Frame, area: Rect, worker: Option<&WorkerView>, color_enabled: bool) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let inner_height = area.height.saturating_sub(2) as usize;

    let title = worker.map(|v| {
        Line::from(vec![
            Span::raw(" logs: "),
            Span::styled(v.name.clone(), styled_if(color_enabled, log_title_style())),
            Span::raw(" "),
        ])
    });

    let mut lines: Vec<Line> = if let Some(v) = worker {
        if v.last_logs.is_empty() {
            vec![Line::from(Span::styled(
                "(no output yet — press s to start, or `workers-dev logs <worker> -f`)",
                styled_if(color_enabled, hint_style()),
            ))]
        } else {
            v.last_logs
                .iter()
                .map(|line| logs::log_line_to_ratatui(line, inner_width, color_enabled))
                .collect()
        }
    } else {
        vec![Line::from("")]
    };

    while lines.len() < inner_height {
        lines.push(Line::from(" ".repeat(inner_width.max(1))));
    }
    lines.truncate(inner_height);

    let mut block = Block::default().borders(Borders::ALL);
    if let Some(title_line) = title {
        block = block.title(title_line);
    } else {
        block = block.title(" logs ");
    }
    let logs = Paragraph::new(lines).block(block);
    f.render_widget(logs, area);
}

fn draw_overlay(f: &mut Frame, area: Rect, message: &str, color_enabled: bool) {
    let popup = centered_rect(60, 20, area);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(message)
            .style(styled_if(color_enabled, confirm_prompt_style()))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" confirm ")
                    .style(styled_if(color_enabled, overlay_bg_style())),
            ),
        popup,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn status_icon(status: &str) -> &'static str {
    match status {
        "connected" => "●",
        "disconnected" | "compiling" => "◐",
        "crashed" => "✗",
        _ => "○",
    }
}
