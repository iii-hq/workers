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

/// Spinner frames cycled through for workers in the compiling state.
const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];
/// Lines of scrollback the log pane can page through (matches the ring buffer).
const LOG_SCROLLBACK: usize = crate::config::LOG_RING_CAPACITY;
/// Lines moved per PageUp/PageDown in the log pane.
const LOG_PAGE: usize = 10;
/// Log-pane height bounds (lines); adjustable with +/-.
const LOG_HEIGHT_MIN: u16 = 6;
const LOG_HEIGHT_MAX: u16 = 60;
const LOG_HEIGHT_DEFAULT: u16 = 18;
/// Minimum height (incl. border + header row) the worker table keeps, so the
/// log pane can never squeeze the primary content off-screen.
const MIN_TABLE_HEIGHT: u16 = 9;
/// Two-column (master/detail) sizing. The worker list is content-fit on the
/// left — 66 cols fits all five columns exactly — and the log pane flexes to
/// fill the rest on the right. Below the combined minimum the two panes stack
/// vertically instead, so neither is crushed on a narrow terminal.
const TABLE_PANE_WIDTH: u16 = 66;
const MIN_LOG_PANE_WIDTH: u16 = 36;
/// Floor for the user-dragged divider (+/-): below this the list loses its
/// less-critical right columns (PID, uptime) to hand width to the logs.
const TABLE_WIDTH_MIN: u16 = 50;
/// 1-col gutter keeps the two pane borders from fusing into a double seam.
const PANE_GUTTER: u16 = 1;
const TWO_COL_MIN_WIDTH: u16 = TABLE_PANE_WIDTH + PANE_GUTTER + MIN_LOG_PANE_WIDTH;

/// Footer help, by available width. The narrowest tier always keeps the two
/// keys a lost user needs (help, quit).
const HELP_FULL: &str =
    " ↑↓ select · s start · x stop · r restart · / filter · f follow · ? keys · q quit ";
const HELP_MID: &str = " s start · x stop · r restart · / filter · ? keys · q quit ";
const HELP_MIN: &str = " / filter · ? keys · q quit ";

enum UiMode {
    Dashboard,
    /// Editing the worker filter; keystrokes append to `filter`.
    Filter,
    /// Full key reference overlay; any key returns to the dashboard.
    Help,
    ConfirmRestart {
        name: String,
        dependents: Vec<String>,
    },
    Busy(String),
}

enum ModeKind {
    Dashboard,
    Filter,
    Help,
    Confirm,
    Busy,
}

fn mode_kind(mode: &UiMode) -> ModeKind {
    match mode {
        UiMode::Dashboard => ModeKind::Dashboard,
        UiMode::Filter => ModeKind::Filter,
        UiMode::Help => ModeKind::Help,
        UiMode::ConfirmRestart { .. } => ModeKind::Confirm,
        UiMode::Busy(_) => ModeKind::Busy,
    }
}

/// Handles the key handlers use to launch worker actions and report their
/// failures back to the UI. Action errors would otherwise vanish: a detached
/// task's `eprintln!` is invisible (or corrupting) under the alt-screen.
struct Actions {
    orchestrator: Arc<Orchestrator>,
    in_flight: Arc<AtomicUsize>,
    errors: tokio::sync::mpsc::UnboundedSender<String>,
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

/// Everything `draw_ui` needs for one frame, bundled to keep the signature sane.
struct UiCtx<'a> {
    engine_url: &'a str,
    views: &'a [WorkerView],
    engine_error: Option<&'a str>,
    display_rows: &'a [DisplayRow],
    mode: &'a UiMode,
    filter: &'a str,
    selected_name: Option<&'a str>,
    log_lines: &'a [String],
    log_scroll: usize,
    follow: bool,
    log_height: u16,
    /// Width of the worker-list pane in the two-column layout (user-dragged
    /// with +/-); ignored in the stacked fallback.
    table_width: u16,
    spinner_frame: usize,
    color_enabled: bool,
    /// Transient action-failure banner shown in the footer.
    error: Option<&'a str>,
}

pub async fn run(orchestrator: Arc<Orchestrator>) -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;

    let result: Result<()> = async {
        let stdout = io::stdout();
        let backend = ratatui::backend::CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Poll the engine on a background task so a slow or unreachable engine
        // query can't freeze keyboard input.
        let (initial_views, initial_err) = orchestrator.dashboard_snapshot().await;
        let (state_tx, mut state_rx) = tokio::sync::watch::channel(DashboardState {
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
                    if state_tx
                        .send(DashboardState {
                            views,
                            engine_error,
                        })
                        .is_err()
                    {
                        break; // UI gone
                    }
                }
            })
        };

        // Number of worker actions still running; gates the Busy overlay and
        // prevents overlapping actions on the same worker.
        let in_flight = Arc::new(AtomicUsize::new(0));
        // Detached worker actions report failures here so the UI can show them
        // (a task's eprintln would be lost/garbled under the alt-screen).
        let (err_tx, mut err_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let actions = Actions {
            orchestrator: orchestrator.clone(),
            in_flight: in_flight.clone(),
            errors: err_tx,
        };

        let mut state = state_rx.borrow().clone();
        let mut table_state = TableState::default();
        let mut mode = UiMode::Dashboard;
        let mut filter = String::new();
        let mut follow = true;
        let mut log_scroll: usize = 0;
        let mut log_height: u16 = LOG_HEIGHT_DEFAULT;
        let mut table_width: u16 = TABLE_PANE_WIDTH;
        let mut spinner_frame: usize = 0;
        let mut error_banner: Option<(String, Instant)> = None;
        let mut running = true;
        let mut needs_redraw = true;
        let mut last_busy_tick = Instant::now();
        let mut last_redraw = Instant::now();

        let color_enabled = orchestrator.config.color_mode.enabled_for_tui();

        {
            let rows = build_display_rows(&state.views, &filter);
            table_state.select(first_worker_row(&rows));
        }

        while running {
            let display_rows = build_display_rows(&state.views, &filter);
            clamp_selection(&mut table_state, &display_rows);

            let compiling = state.views.iter().any(|v| v.display_status == "compiling");

            if needs_redraw {
                if compiling {
                    spinner_frame = spinner_frame.wrapping_add(1);
                }
                let selected_name =
                    selected_worker(&display_rows, &state.views, table_state.selected());
                let log_lines = match &selected_name {
                    Some(n) => orchestrator
                        .logs_tail(n, LOG_SCROLLBACK)
                        .await
                        .unwrap_or_default(),
                    None => Vec::new(),
                };
                let ctx = UiCtx {
                    engine_url: &orchestrator.config.engine_url,
                    views: &state.views,
                    engine_error: state.engine_error.as_deref(),
                    display_rows: &display_rows,
                    mode: &mode,
                    filter: &filter,
                    selected_name: selected_name.as_deref(),
                    log_lines: &log_lines,
                    log_scroll,
                    follow,
                    log_height,
                    table_width,
                    spinner_frame,
                    color_enabled,
                    error: error_banner.as_ref().map(|(s, _)| s.as_str()),
                };
                terminal.draw(|f| draw_ui(f, &mut table_state, &ctx))?;
                needs_redraw = false;
                last_redraw = Instant::now();
            }

            // Clear the Busy overlay once actions finish (on the poll cadence so
            // brief informational messages stay readable), never while running.
            if last_busy_tick.elapsed() >= poll_interval {
                last_busy_tick = Instant::now();
                if matches!(mode, UiMode::Busy(_)) && in_flight.load(Ordering::SeqCst) == 0 {
                    mode = UiMode::Dashboard;
                    needs_redraw = true;
                }
            }

            // Animate the spinner while compiling and refresh live logs while
            // following; otherwise stay idle until something actually changes.
            let spinner_due = compiling && last_redraw.elapsed() >= Duration::from_millis(120);
            let live_logs_due = follow
                && matches!(mode_kind(&mode), ModeKind::Dashboard | ModeKind::Filter)
                && last_redraw.elapsed() >= Duration::from_millis(500);
            if spinner_due || live_logs_due {
                needs_redraw = true;
            }

            if event::poll(Duration::from_millis(120))? {
                match event::read()? {
                    Event::Key(key) => {
                        needs_redraw = true;
                        error_banner = None; // any keypress acknowledges the banner
                        match mode_kind(&mode) {
                            ModeKind::Filter => {
                                handle_filter_key(
                                    key,
                                    &mut filter,
                                    &mut mode,
                                    &mut table_state,
                                    &state.views,
                                );
                            }
                            ModeKind::Help => mode = UiMode::Dashboard, // any key closes help
                            ModeKind::Confirm => handle_confirm_key(key, &mut mode, &actions),
                            ModeKind::Busy => {
                                if key.code == KeyCode::Esc && in_flight.load(Ordering::SeqCst) == 0
                                {
                                    mode = UiMode::Dashboard;
                                }
                            }
                            ModeKind::Dashboard => {
                                let two_col = terminal
                                    .size()
                                    .map(|s| s.width >= TWO_COL_MIN_WIDTH)
                                    .unwrap_or(true);
                                running = handle_dashboard_key(
                                    &actions,
                                    key,
                                    &display_rows,
                                    &state.views,
                                    &mut table_state,
                                    &mut mode,
                                    &mut filter,
                                    &mut follow,
                                    &mut log_scroll,
                                    &mut log_height,
                                    &mut table_width,
                                    two_col,
                                );
                            }
                        }
                    }
                    // Repaint on resize so the layout reflows immediately.
                    Event::Resize(_, _) => needs_redraw = true,
                    _ => {}
                }
            }

            // Surface any action failure as a footer banner; expire it after 8s.
            while let Ok(err) = err_rx.try_recv() {
                error_banner = Some((err, Instant::now()));
                needs_redraw = true;
            }
            if matches!(&error_banner, Some((_, t)) if t.elapsed() >= Duration::from_secs(8)) {
                error_banner = None;
                needs_redraw = true;
            }

            if state_rx.has_changed().unwrap_or(false) {
                state = state_rx.borrow_and_update().clone();
                needs_redraw = true;
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
fn handle_dashboard_key(
    actions: &Actions,
    key: KeyEvent,
    display_rows: &[DisplayRow],
    views: &[WorkerView],
    table_state: &mut TableState,
    mode: &mut UiMode,
    filter: &mut String,
    follow: &mut bool,
    log_scroll: &mut usize,
    log_height: &mut u16,
    table_width: &mut u16,
    two_col: bool,
) -> bool {
    let selected = table_state.selected();
    let worker_name = selected_worker(display_rows, views, selected);

    match key.code {
        KeyCode::Char('q') => return false,
        // Esc clears an active filter first; only quits when nothing to clear.
        KeyCode::Esc => {
            if filter.is_empty() {
                return false;
            }
            filter.clear();
        }
        KeyCode::Up | KeyCode::Down | KeyCode::Char('k') | KeyCode::Char('j') => {
            let down = matches!(key.code, KeyCode::Down | KeyCode::Char('j'));
            if let Some(i) = move_selection(display_rows, selected.unwrap_or(0), down) {
                table_state.select(Some(i));
                // Re-follow the newly selected worker's live tail.
                *follow = true;
                *log_scroll = 0;
            }
        }
        KeyCode::Char('/') => *mode = UiMode::Filter,
        KeyCode::Char('?') => *mode = UiMode::Help,
        KeyCode::Char('f') => {
            *follow = !*follow;
            if *follow {
                *log_scroll = 0;
            }
        }
        KeyCode::PageUp => {
            *follow = false;
            *log_scroll = (*log_scroll + LOG_PAGE).min(LOG_SCROLLBACK);
        }
        KeyCode::PageDown => {
            *log_scroll = log_scroll.saturating_sub(LOG_PAGE);
            if *log_scroll == 0 {
                *follow = true;
            }
        }
        // `+` always gives the logs more room. Two-column: drag the divider left
        // (shrink the list). Stacked: grow the log pane's height. `-` reverses it.
        KeyCode::Char('+') | KeyCode::Char('=') => {
            if two_col {
                *table_width = table_width.saturating_sub(4).max(TABLE_WIDTH_MIN);
            } else {
                *log_height = (*log_height + 2).min(LOG_HEIGHT_MAX);
            }
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            if two_col {
                *table_width = (*table_width + 4).min(TABLE_PANE_WIDTH);
            } else {
                *log_height = log_height.saturating_sub(2).max(LOG_HEIGHT_MIN);
            }
        }
        KeyCode::Char('r') => {
            if let Some(name) = worker_name {
                if views.iter().any(|v| v.name == name && v.spawnable) {
                    let dependents = actions
                        .orchestrator
                        .graph
                        .reverse_dependents(&name)
                        .unwrap_or_default();
                    *mode = UiMode::ConfirmRestart { name, dependents };
                } else {
                    *mode = not_startable_msg();
                }
            }
        }
        KeyCode::Char('s') => {
            if let Some(name) = worker_name {
                if views.iter().any(|v| v.name == name && v.spawnable) {
                    spawn_start(actions, vec![name.clone()]);
                    *mode = UiMode::Busy(format!("starting {name}…"));
                } else {
                    *mode = not_startable_msg();
                }
            }
        }
        KeyCode::Char('x') => {
            if let Some(name) = worker_name {
                // Only stop workers with a live local child; `external` and
                // `elsewhere` rows have no process workers-dev can kill.
                if views.iter().any(|v| v.name == name && v.local_pid.is_some()) {
                    spawn_stop(actions, vec![name.clone()]);
                    *mode = UiMode::Busy(format!("stopping {name}…"));
                } else {
                    *mode = UiMode::Busy("nothing to stop: not running under workers-dev".into());
                }
            }
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            spawn_start_harness_stack(actions);
            *mode = UiMode::Busy("starting harness stack…".to_string());
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let names = actions.orchestrator.config.workers.clone();
            spawn_start(actions, names);
            *mode = UiMode::Busy("starting all managed workers…".to_string());
        }
        _ => {}
    }
    true
}

fn handle_filter_key(
    key: KeyEvent,
    filter: &mut String,
    mode: &mut UiMode,
    table_state: &mut TableState,
    views: &[WorkerView],
) {
    match key.code {
        KeyCode::Enter => *mode = UiMode::Dashboard, // keep the filter
        KeyCode::Esc => {
            filter.clear();
            *mode = UiMode::Dashboard;
        }
        KeyCode::Backspace => {
            filter.pop();
        }
        KeyCode::Char(c) => filter.push(c),
        _ => {}
    }
    // Keep the highlight on a visible row as the match set changes.
    let rows = build_display_rows(views, filter);
    table_state.select(first_worker_row(&rows));
}

fn handle_confirm_key(key: KeyEvent, mode: &mut UiMode, actions: &Actions) {
    let UiMode::ConfirmRestart { name, .. } = mode else {
        return;
    };
    let name = name.clone();
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            spawn_restart(actions, name.clone());
            *mode = UiMode::Busy(format!("restarting {name} + dependents…"));
        }
        KeyCode::Char('n') | KeyCode::Esc => *mode = UiMode::Dashboard,
        _ => {}
    }
}

fn not_startable_msg() -> UiMode {
    UiMode::Busy("worker not startable from workers-dev (use iii worker add)".into())
}

/// Run a worker action on a detached task, counting it as in-flight for the
/// whole duration so the UI keeps the Busy overlay up and refuses overlapping
/// actions until it finishes. Failures are sent back to the UI (not printed,
/// which would corrupt the alt-screen).
fn spawn_action<F>(actions: &Actions, fut: F)
where
    F: std::future::Future<Output = Result<()>> + Send + 'static,
{
    actions.in_flight.fetch_add(1, Ordering::SeqCst);
    let in_flight = actions.in_flight.clone();
    let errors = actions.errors.clone();
    tokio::spawn(async move {
        if let Err(err) = fut.await {
            let _ = errors.send(format!("{err:#}"));
        }
        in_flight.fetch_sub(1, Ordering::SeqCst);
    });
}

fn spawn_start(actions: &Actions, names: Vec<String>) {
    let orchestrator = actions.orchestrator.clone();
    spawn_action(actions, async move {
        orchestrator.start_workers(&names, false).await
    });
}

fn spawn_start_harness_stack(actions: &Actions) {
    let orchestrator = actions.orchestrator.clone();
    spawn_action(actions, async move {
        orchestrator.start_harness_stack(false).await
    });
}

fn spawn_stop(actions: &Actions, names: Vec<String>) {
    let orchestrator = actions.orchestrator.clone();
    spawn_action(
        actions,
        async move { orchestrator.stop_workers(&names).await },
    );
}

fn spawn_restart(actions: &Actions, worker: String) {
    let orchestrator = actions.orchestrator.clone();
    spawn_action(actions, async move {
        orchestrator.restart_worker(&worker).await
    });
}

/// Build the table rows, applying the name filter (case-insensitive). A group
/// header is emitted only when that group has at least one matching worker.
fn build_display_rows(views: &[WorkerView], filter: &str) -> Vec<DisplayRow> {
    let needle = filter.to_lowercase();
    let mut rows = Vec::new();
    let mut last_group = None;
    for (idx, view) in views.iter().enumerate() {
        if !needle.is_empty() && !view.name.to_lowercase().contains(&needle) {
            continue;
        }
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
    display_rows
        .iter()
        .position(|row| matches!(row.kind, DisplayRowKind::Worker(_)))
}

/// Snap the selection back onto a visible worker row when the current one
/// scrolled out (e.g. the filter changed) so the highlight is never stranded.
fn clamp_selection(table_state: &mut TableState, display_rows: &[DisplayRow]) {
    let valid = matches!(
        table_state.selected().and_then(|i| display_rows.get(i)),
        Some(DisplayRow {
            kind: DisplayRowKind::Worker(_)
        })
    );
    if !valid {
        table_state.select(first_worker_row(display_rows));
    }
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
    row: Option<usize>,
) -> Option<String> {
    match display_rows.get(row?)?.kind {
        DisplayRowKind::Worker(idx) => views.get(idx).map(|v| v.name.clone()),
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

fn draw_ui(f: &mut Frame, table_state: &mut TableState, ctx: &UiCtx) {
    let area = f.area();
    // Header and footer span full width; the body between them carries the
    // two panes.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(MIN_TABLE_HEIGHT),
            Constraint::Length(1),
        ])
        .split(area);
    let body = chunks[1];

    draw_header(f, chunks[0], ctx);

    if body.width >= TWO_COL_MIN_WIDTH {
        // Master/detail: the worker list (content-fit) on the left, its logs
        // flexing to fill the rest on the right. Wide terminals give the logs
        // the horizontal room they actually benefit from instead of squeezing
        // them into a short strip under a half-empty table.
        // Divider position: the user's preferred list width (+/-), never so wide
        // it starves the logs below their minimum.
        let table_w = ctx
            .table_width
            .clamp(TABLE_WIDTH_MIN, TABLE_PANE_WIDTH)
            .min(body.width.saturating_sub(MIN_LOG_PANE_WIDTH + PANE_GUTTER));
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(table_w),
                Constraint::Length(PANE_GUTTER),
                Constraint::Min(MIN_LOG_PANE_WIDTH),
            ])
            .split(body);
        draw_table(f, cols[0], table_state, ctx);
        draw_log_pane(f, cols[2], ctx);
    } else {
        // Too narrow for two columns: stack them, with a user-resizable (+/-)
        // log pane that can never starve the list below MIN_TABLE_HEIGHT.
        let log_h = ctx
            .log_height
            .min(body.height.saturating_sub(MIN_TABLE_HEIGHT))
            .max(LOG_HEIGHT_MIN.min(body.height));
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(MIN_TABLE_HEIGHT), Constraint::Length(log_h)])
            .split(body);
        draw_table(f, rows[0], table_state, ctx);
        draw_log_pane(f, rows[1], ctx);
    }

    draw_footer(f, chunks[2], ctx);

    // Modals center over the whole content area so they keep their width in
    // either layout.
    if let UiMode::ConfirmRestart { name, dependents } = ctx.mode {
        draw_confirm_overlay(f, body, name, dependents, ctx.color_enabled);
    }
    if matches!(ctx.mode, UiMode::Help) {
        draw_help_overlay(f, area, ctx.color_enabled);
    }
}

fn draw_header(f: &mut Frame, area: Rect, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let mut spans = vec![
        Span::styled(
            "workers-dev",
            styled_if(color, header_accent_style()).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(ctx.engine_url, styled_if(color, engine_url_style())),
    ];

    if ctx.engine_error.is_some() {
        spans.push(Span::styled(
            "  ⚠ unreachable",
            styled_if(color, Style::default().fg(Color::Red)).add_modifier(Modifier::BOLD),
        ));
    } else {
        let (mut connected, mut compiling, mut crashed, mut stopped) = (0u32, 0u32, 0u32, 0u32);
        for v in ctx.views {
            match v.display_status.as_str() {
                "connected" => connected += 1,
                "compiling" => compiling += 1,
                "crashed" => crashed += 1,
                _ => stopped += 1,
            }
        }
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            format!("●{connected}"),
            styled_if(color, Style::default().fg(Color::Green)),
        ));
        if compiling > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("◐{compiling}"),
                styled_if(color, Style::default().fg(Color::Yellow)),
            ));
        }
        if crashed > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("✗{crashed}"),
                styled_if(color, Style::default().fg(Color::Red)),
            ));
        }
        if stopped > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("○{stopped}"),
                styled_if(color, muted_cell_style()),
            ));
        }
    }

    if !ctx.filter.is_empty() {
        spans.push(Span::styled(
            format!("   filter:{}", ctx.filter),
            styled_if(color, Style::default().fg(Color::Yellow)),
        ));
    }

    // No box title: the accent "workers-dev" span already names the pane.
    let header = Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::ALL));
    f.render_widget(header, area);
}

fn draw_table(f: &mut Frame, area: Rect, table_state: &mut TableState, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    if ctx.display_rows.is_empty() {
        let msg = if ctx.filter.is_empty() {
            "(no workers discovered)".to_string()
        } else {
            format!("no workers match \"{}\"  ·  Esc to clear", ctx.filter)
        };
        let placeholder = Paragraph::new(Line::from(Span::styled(
            format!("  {msg}"),
            styled_if(color, hint_style()),
        )))
        .block(Block::default().borders(Borders::ALL).title(" Workers "));
        f.render_widget(placeholder, area);
        return;
    }
    let rows: Vec<Row> = ctx
        .display_rows
        .iter()
        .map(|row| match row.kind {
            DisplayRowKind::Header(group) => {
                Row::new(vec![Cell::from(format!("── {} ──", group.label()))
                    .style(styled_if(color, group_header_style(group)))])
            }
            DisplayRowKind::Worker(idx) => {
                let v = &ctx.views[idx];
                let icon = status_icon(&v.display_status, ctx.spinner_frame);
                // Name cell is just glyph + name. The wide "(iii worker add)"
                // label and the crash exit code moved to the Process column, so
                // the name sits right next to its status instead of behind a
                // column sized for the longest label.
                let name_cell = Cell::from(Span::styled(
                    format!("{icon} {}", v.name),
                    styled_if(color, status_style(&v.display_status)),
                ));
                let (process_text, process_st) = process_cell(v, color);
                let pid = v
                    .local_pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "—".to_string());
                Row::new(vec![
                    name_cell,
                    Cell::from(Span::styled(process_text, process_st)),
                    Cell::from(Span::styled(
                        v.engine_status.clone(),
                        styled_if(color, engine_style(&v.engine_status)),
                    )),
                    Cell::from(Span::styled(
                        pid.clone(),
                        styled_if(color, muted_cell_style_for(&pid)),
                    )),
                    Cell::from(Span::styled(
                        v.uptime.clone(),
                        styled_if(color, muted_cell_style_for(&v.uptime)),
                    )),
                ])
            }
        })
        .collect();

    // Content-fit, left-packed widths: the worker name sits right next to its
    // status. Worker only has to fit a name now (label/exit moved to Process),
    // so it's tight; Uptime (Min) absorbs the right slack.
    let table = Table::new(
        rows,
        [
            Constraint::Length(24),
            Constraint::Length(10),
            Constraint::Length(11),
            Constraint::Length(7),
            Constraint::Min(8),
        ],
    )
    .header(
        Row::new(vec!["Worker", "Process", "Engine", "PID", "Uptime"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title(" Workers "))
    .row_highlight_style(styled_if(color, selection_row_style()));
    f.render_stateful_widget(table, area, table_state);
}

/// Process-column text + style. Carries the management/crash detail that used
/// to crowd the name cell: `external` for workers this tool doesn't start,
/// `elsewhere` for ones connected to the engine but started outside this tool,
/// and the exit code on a crash.
fn process_cell(v: &WorkerView, color: bool) -> (String, Style) {
    if !v.spawnable {
        return (
            "external".to_string(),
            styled_if(color, non_spawnable_style()),
        );
    }
    if v.process_status == "crashed" {
        let txt = v
            .exit_code
            .map(|n| format!("exit {n}"))
            .unwrap_or_else(|| "crashed".to_string());
        return (txt, styled_if(color, Style::default().fg(Color::Red)));
    }
    // Connected to the engine but with no process of ours: it's running, just
    // started elsewhere. Say so rather than printing "stopped" beside the
    // engine's "connected", which reads as a contradiction.
    if v.process_status == "stopped" && v.engine_status == "connected" {
        return (
            "elsewhere".to_string(),
            styled_if(color, non_spawnable_style()),
        );
    }
    (
        v.process_status.clone(),
        styled_if(color, process_style(&v.process_status)),
    )
}

fn muted_cell_style_for(value: &str) -> Style {
    if value == "—" {
        muted_cell_style()
    } else {
        Style::default()
    }
}

fn draw_log_pane(f: &mut Frame, area: Rect, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let inner_width = area.width.saturating_sub(2) as usize;
    let inner_height = area.height.saturating_sub(2) as usize;

    let total = ctx.log_lines.len();
    let max_scroll = total.saturating_sub(inner_height);
    let scroll = if ctx.follow {
        0
    } else {
        ctx.log_scroll.min(max_scroll)
    };
    let end = total.saturating_sub(scroll);
    let start = end.saturating_sub(inner_height);

    let mut lines: Vec<Line> = if total == 0 {
        // No selection (e.g. the filter hid every worker) needs a different hint
        // than a selected worker that simply hasn't produced output yet.
        let msg = if ctx.selected_name.is_none() {
            "(select a worker to view logs)"
        } else {
            "(no output yet — press s to start this worker, or Ctrl+u to start the harness stack)"
        };
        vec![Line::from(Span::styled(msg, styled_if(color, hint_style())))]
    } else {
        ctx.log_lines[start..end]
            .iter()
            .map(|line| logs::log_line_to_ratatui(line, inner_width, color))
            .collect()
    };
    while lines.len() < inner_height {
        lines.push(Line::from(" ".repeat(inner_width.max(1))));
    }
    lines.truncate(inner_height.max(1));

    let mut title = vec![Span::raw(" logs")];
    if let Some(name) = ctx.selected_name {
        title.push(Span::raw(": "));
        title.push(Span::styled(
            name.to_string(),
            styled_if(color, log_title_style()),
        ));
    }
    title.push(Span::raw("  "));
    if ctx.follow {
        title.push(Span::styled(
            "▶ live ",
            styled_if(color, Style::default().fg(Color::Green)),
        ));
    } else {
        title.push(Span::styled(
            format!("⏸ scrolled +{scroll} "),
            styled_if(color, Style::default().fg(Color::Yellow)),
        ));
    }

    let pane = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(Line::from(title)),
    );
    f.render_widget(pane, area);
}

fn draw_footer(f: &mut Frame, area: Rect, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    if let Some(err) = ctx.error {
        let banner = Paragraph::new(format!(" ⚠ {err} "))
            .style(styled_if(color, Style::default().fg(Color::Red)).add_modifier(Modifier::BOLD));
        f.render_widget(banner, area);
        return;
    }
    let (text, style) = match ctx.mode {
        UiMode::Busy(msg) => (msg.clone(), styled_if(color, footer_style())),
        UiMode::Filter => (
            format!(" filter: {}_   (Enter apply · Esc clear) ", ctx.filter),
            styled_if(color, Style::default().fg(Color::Yellow)),
        ),
        _ => {
            let help = if area.width >= 86 {
                HELP_FULL
            } else if area.width >= 64 {
                HELP_MID
            } else {
                HELP_MIN
            };
            (help.to_string(), styled_if(color, footer_style()))
        }
    };
    f.render_widget(Paragraph::new(text).style(style), area);
}

fn draw_confirm_overlay(
    f: &mut Frame,
    area: Rect,
    name: &str,
    dependents: &[String],
    color_enabled: bool,
) {
    let mut lines = vec![Line::from(format!("Restart {name} and its dependents?"))];
    if dependents.is_empty() {
        lines.push(Line::from(Span::styled(
            "no dependents — only this worker restarts",
            styled_if(color_enabled, hint_style()),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!("also restarts: {}", dependents.join(", ")),
            styled_if(color_enabled, Style::default().fg(Color::Yellow)),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from("y/Enter  yes        n/Esc  no"));

    let popup = centered_rect(64, 40, area);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines)
            .style(styled_if(color_enabled, confirm_prompt_style()))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" confirm restart ")
                    .style(styled_if(color_enabled, overlay_bg_style())),
            ),
        popup,
    );
}

fn draw_help_overlay(f: &mut Frame, area: Rect, color: bool) {
    let keys = [
        ("↑ ↓  k j", "select worker"),
        ("s", "start selected worker"),
        ("x", "stop selected worker"),
        ("r", "restart selected + dependents"),
        ("f", "toggle live log follow"),
        ("PgUp PgDn", "scroll logs"),
        ("+ -", "resize the log pane"),
        ("/", "filter workers by name"),
        ("Ctrl+u", "start the harness stack"),
        ("Ctrl+a", "start all managed workers"),
        ("?", "toggle this help"),
        ("q", "quit (workers keep running)"),
    ];
    let mut lines = vec![Line::from("")];
    for (k, d) in keys {
        lines.push(Line::from(vec![
            Span::styled(
                format!("   {k:<12}"),
                styled_if(color, Style::default().fg(Color::Cyan)),
            ),
            Span::raw(d),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "   status",
        styled_if(color, Style::default().add_modifier(Modifier::BOLD)),
    )));
    lines.push(Line::from(vec![
        Span::styled("   ● ", styled_if(color, Style::default().fg(Color::Green))),
        Span::raw("connected     "),
        Span::styled("◐ ", styled_if(color, Style::default().fg(Color::Yellow))),
        Span::raw("compiling / disconnected"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("   ✗ ", styled_if(color, Style::default().fg(Color::Red))),
        Span::raw("crashed       "),
        Span::styled("○ ", styled_if(color, muted_cell_style())),
        Span::raw("stopped"),
    ]));
    lines.push(Line::from(Span::styled(
        "   external  = installed via `iii worker add`",
        styled_if(color, hint_style()),
    )));
    lines.push(Line::from(Span::styled(
        "   elsewhere = connected, started outside workers-dev",
        styled_if(color, hint_style()),
    )));
    let popup = centered_rect(58, 75, area);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" keys · any key to close ")
                .style(styled_if(color, overlay_bg_style())),
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

fn status_icon(status: &str, spinner_frame: usize) -> &'static str {
    match status {
        "connected" => "●",
        "compiling" => SPINNER[spinner_frame % SPINNER.len()],
        "disconnected" => "◐",
        "crashed" => "✗",
        _ => "○",
    }
}
