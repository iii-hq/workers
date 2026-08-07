mod stacks;
mod theme;

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
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
    branch_style, confirm_prompt_style, engine_style, engine_url_style, footer_style,
    group_header_style, header_accent_style, hint_style, log_title_style, muted_cell_style,
    non_spawnable_style, overlay_bg_style, process_style, selection_row_style, status_style,
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
/// left — 73 cols fits all six columns (incl. UI) exactly — and the log pane
/// flexes to fill the rest on the right. Below the combined minimum the two
/// panes stack vertically instead, so neither is crushed on a narrow terminal.
const TABLE_PANE_WIDTH: u16 = 73;
const MIN_LOG_PANE_WIDTH: u16 = 36;
/// Floor for the user-dragged divider (+/-): below this the list loses its
/// less-critical right columns (PID, uptime) to hand width to the logs.
const TABLE_WIDTH_MIN: u16 = 50;
/// 1-col gutter keeps the two pane borders from fusing into a double seam.
const PANE_GUTTER: u16 = 1;
const TWO_COL_MIN_WIDTH: u16 = TABLE_PANE_WIDTH + PANE_GUTTER + MIN_LOG_PANE_WIDTH;
/// Longest branch name shown in the header before truncation, so a
/// pathological name can't push the health summary out of view.
const BRANCH_MAX: usize = 40;
/// How long the `e` (start engine) action waits for the engine to become
/// reachable before reporting failure. The Busy overlay blocks all input
/// while an action runs, so this is kept short: a dead `iii` is detected
/// within a poll tick, a healthy engine binds within a couple of seconds,
/// and a slow one keeps coming up in the background where the poller shows it.
const ENGINE_START_WAIT_MS: u64 = 8_000;

/// Footer help, by available width. The narrowest tier always keeps the two
/// keys a lost user needs (help, quit).
const HELP_FULL: &str =
    " ↑↓ select · Space mark · s start · x stop · r restart · w ui-watch · / filter · f follow · ? keys · q quit ";
const HELP_MID: &str = " s start · x stop · r restart · / filter · ? keys · q quit ";
const HELP_MIN: &str = " / filter · ? keys · q quit ";

enum UiMode {
    Dashboard,
    /// Editing the worker filter; keystrokes append to `filter`.
    Filter,
    /// Full key reference overlay; any key returns to the dashboard.
    Help,
    /// Dependency inspector for one worker; any key returns to the dashboard.
    /// Statuses are looked up live at draw time, so the overlay stays current.
    Deps {
        name: String,
        deps: Vec<String>,
        dependents: Vec<String>,
    },
    ConfirmRestart {
        name: String,
        dependents: Vec<String>,
    },
    Busy(String),
    /// Stack picker (Ctrl+u with several stacks defined). `selected` indexes
    /// into config.stacks; rows with no startable roots are skipped.
    StackPicker {
        selected: usize,
    },
}

enum ModeKind {
    Dashboard,
    Filter,
    Help,
    Deps,
    Confirm,
    Busy,
    StackPicker,
}

fn mode_kind(mode: &UiMode) -> ModeKind {
    match mode {
        UiMode::Dashboard => ModeKind::Dashboard,
        UiMode::Filter => ModeKind::Filter,
        UiMode::Help => ModeKind::Help,
        UiMode::Deps { .. } => ModeKind::Deps,
        UiMode::ConfirmRestart { .. } => ModeKind::Confirm,
        UiMode::Busy(_) => ModeKind::Busy,
        UiMode::StackPicker { .. } => ModeKind::StackPicker,
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
    /// Current branch of the repo this instance orchestrates, so multiple
    /// checkouts/worktrees are easy to tell apart. Re-read on the poll cadence
    /// to track mid-session branch switches; None when repo isn't a checkout.
    repo_branch: Option<String>,
}

enum DisplayRowKind {
    Header(WorkerGroup, usize),
    Worker(usize),
}

struct DisplayRow {
    kind: DisplayRowKind,
}

/// Everything `draw_ui` needs for one frame, bundled to keep the signature sane.
struct UiCtx<'a> {
    engine_url: &'a str,
    repo_branch: Option<&'a str>,
    current_stack: &'a str,
    default_stack: &'a str,
    stacks: &'a [(String, Vec<String>)],
    views: &'a [WorkerView],
    /// Workers marked with Space for a new stack (Task 5 saves these).
    marked: &'a std::collections::HashSet<String>,
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
    // Save the terminal's title (XTWINOPS push; ignored where unsupported)
    // before set_terminal_title overwrites it, so quitting can restore it.
    execute!(io::stdout(), EnterAlternateScreen, Print("\x1b[22;0t"))?;

    let result: Result<()> = async {
        let stdout = io::stdout();
        let backend = ratatui::backend::CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Poll the engine on a background task so a slow or unreachable engine
        // query can't freeze keyboard input.
        let mut current_stack = orchestrator.config.default_stack.clone();
        let initial_members = orchestrator.stack_members(&current_stack)?;
        let (initial_views, initial_err) = orchestrator.dashboard_snapshot(&initial_members).await;
        // The poller reads the current stack's member set each tick, so a
        // switch regroups from the next poll onward (the switch itself also
        // regroups the local snapshot immediately).
        let shared_members = Arc::new(std::sync::RwLock::new(initial_members));
        let initial_branch = crate::git::current_branch(&orchestrator.config.repo_root);
        set_terminal_title(initial_branch.as_deref());
        let (state_tx, mut state_rx) = tokio::sync::watch::channel(DashboardState {
            views: initial_views,
            engine_error: initial_err,
            repo_branch: initial_branch,
        });
        let poll_interval = Duration::from_millis(orchestrator.config.poll_interval_ms);
        let poller = {
            let orchestrator = orchestrator.clone();
            let shared_members = shared_members.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(poll_interval).await;
                    let members = shared_members.read().expect("members lock").clone();
                    let (views, engine_error) = orchestrator.dashboard_snapshot(&members).await;
                    if state_tx
                        .send(DashboardState {
                            views,
                            engine_error,
                            repo_branch: crate::git::current_branch(&orchestrator.config.repo_root),
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
        // Workers marked with Space, held by name so a mark survives filtering
        // and re-sorting. Cleared after a successful save.
        let mut marked: std::collections::HashSet<String> = std::collections::HashSet::new();
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
                if compiling || matches!(mode, UiMode::Busy(_)) {
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
                    repo_branch: state.repo_branch.as_deref(),
                    current_stack: &current_stack,
                    default_stack: &orchestrator.config.default_stack,
                    stacks: &orchestrator.config.stacks,
                    views: &state.views,
                    marked: &marked,
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

            // Animate the spinner while compiling or an action runs (Busy
            // dialog) and refresh live logs while following; otherwise stay
            // idle until something actually changes.
            let spinner_due = (compiling || matches!(mode, UiMode::Busy(_)))
                && last_redraw.elapsed() >= Duration::from_millis(120);
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
                            // Any key closes the help / dependency overlays.
                            ModeKind::Help | ModeKind::Deps => mode = UiMode::Dashboard,
                            ModeKind::Confirm => handle_confirm_key(key, &mut mode, &actions),
                            ModeKind::Busy => {
                                if key.code == KeyCode::Esc && in_flight.load(Ordering::SeqCst) == 0
                                {
                                    mode = UiMode::Dashboard;
                                }
                            }
                            ModeKind::StackPicker => {
                                // Selection may regroup the list; keep the
                                // highlight on the same worker by name.
                                let keep_name = selected_worker(
                                    &display_rows,
                                    &state.views,
                                    table_state.selected(),
                                );
                                if let Some(name) = stacks::handle_stack_picker_key(
                                    key,
                                    &mut mode,
                                    &orchestrator.config.stacks,
                                ) {
                                    // Only commit to the new stack — label,
                                    // grouping, Busy, and the start — once
                                    // its members actually resolve; an error
                                    // leaves current_stack/shared_members/mode/
                                    // the spawned action all untouched.
                                    match orchestrator.stack_members(&name) {
                                        Ok(members) => {
                                            current_stack = name.clone();
                                            // Mutates (re-sorts) state.views;
                                            // the outer `display_rows` from
                                            // the top of this loop iteration
                                            // indexes the pre-sort order and
                                            // must not be read again below —
                                            // nothing does today.
                                            crate::status::assign_view_groups(
                                                &mut state.views,
                                                &members,
                                            );
                                            *shared_members.write().expect("members lock") =
                                                members;
                                            let rows = build_display_rows(&state.views, &filter);
                                            let target = keep_name
                                                .as_deref()
                                                .and_then(|n| row_of_worker(&rows, &state.views, n))
                                                .or_else(|| first_worker_row(&rows));
                                            table_state.select(target);
                                            mode = UiMode::Busy(format!("starting stack {name}…"));
                                            spawn_start_stack(&actions, name);
                                        }
                                        Err(err) => {
                                            error_banner =
                                                Some((format!("{err:#}"), Instant::now()));
                                        }
                                    }
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
                                    &mut marked,
                                    &mut follow,
                                    &mut log_scroll,
                                    &mut log_height,
                                    &mut table_width,
                                    &current_stack,
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
                let fresh = state_rx.borrow_and_update().clone();
                // Keep the terminal/tmux pane title tracking the branch.
                if fresh.repo_branch != state.repo_branch {
                    set_terminal_title(fresh.repo_branch.as_deref());
                }
                state = fresh;
                // The poller reads `shared_members` before its (possibly
                // slow) `dashboard_snapshot` await, so a stack switch mid-await
                // lands here grouped by the stale set. Re-group against the
                // authoritative current set so a switch never flashes the old
                // stack's grouping/header for a frame. Read-and-drop within
                // this statement only — never held across an `.await`.
                crate::status::assign_view_groups(
                    &mut state.views,
                    &shared_members.read().expect("members lock"),
                );
                needs_redraw = true;
            }
        }

        poller.abort();
        Ok(())
    }
    .await;

    let _ = disable_raw_mode();
    // Clear our title, then pop the saved one: terminals with a title stack
    // restore the original; elsewhere the title at least stops claiming a
    // workers-dev instance that no longer exists.
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        SetTitle(""),
        Print("\x1b[23;0t")
    );
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
    marked: &mut std::collections::HashSet<String>,
    follow: &mut bool,
    log_scroll: &mut usize,
    log_height: &mut u16,
    table_width: &mut u16,
    current_stack: &str,
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
        // Jump to the first/last worker — the list runs to ~50 rows, and
        // stepping there one `j` at a time is a chore.
        KeyCode::Char('g') | KeyCode::Home | KeyCode::Char('G') | KeyCode::End => {
            let target = if matches!(key.code, KeyCode::Char('g') | KeyCode::Home) {
                first_worker_row(display_rows)
            } else {
                display_rows
                    .iter()
                    .rposition(|row| matches!(row.kind, DisplayRowKind::Worker(_)))
            };
            // Like Up/Down, only re-follow when the selection actually moves —
            // `g` on the first row shouldn't yank a scrolled log pane back down.
            if target.is_some() && target != selected {
                table_state.select(target);
                *follow = true;
                *log_scroll = 0;
            }
        }
        // Mark for stack creation (n). Marks are independent of selection and
        // of the filter — the name prompt shows what will be saved.
        KeyCode::Char(' ') => {
            if let Some(name) = worker_name {
                if !marked.remove(&name) {
                    marked.insert(name);
                }
            }
        }
        KeyCode::Char('/') => *mode = UiMode::Filter,
        KeyCode::Char('?') => *mode = UiMode::Help,
        KeyCode::Char('d') => {
            if let Some(name) = worker_name {
                let mut deps = actions.orchestrator.graph.dependencies(&name).to_vec();
                deps.sort_unstable();
                let dependents = actions
                    .orchestrator
                    .graph
                    .reverse_dependents(&name)
                    .unwrap_or_default();
                *mode = UiMode::Deps {
                    name,
                    deps,
                    dependents,
                };
            }
        }
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
        // Injectable-UI watcher mode (SOP dev loop): flip `pnpm watch` +
        // `III_<WORKER>_UI_WATCH=1` for the selected worker. A running worker
        // restarts to apply the env var; a stopped one picks it up next start.
        KeyCode::Char('w') => {
            if let Some(name) = worker_name {
                match views
                    .iter()
                    .find(|v| v.name == name)
                    .and_then(|v| v.ui_watch)
                {
                    None => {
                        *mode =
                            UiMode::Busy(format!("{name} ships no ui/ project — nothing to watch"));
                    }
                    Some(currently_on) => {
                        spawn_toggle_ui_watch(actions, name.clone());
                        *mode = UiMode::Busy(if currently_on {
                            format!("disabling ui watch for {name}…")
                        } else {
                            format!("enabling ui watch for {name}…")
                        });
                    }
                }
            }
        }
        KeyCode::Char('x') => {
            if let Some(name) = worker_name {
                // Only stop workers with a live local child; `external` and
                // `elsewhere` rows have no process workers-dev can kill.
                if views
                    .iter()
                    .any(|v| v.name == name && v.local_pid.is_some())
                {
                    spawn_stop(actions, vec![name.clone()]);
                    *mode = UiMode::Busy(format!("stopping {name}…"));
                } else {
                    *mode = UiMode::Busy("nothing to stop: not running under workers-dev".into());
                }
            }
        }
        KeyCode::Char('e') => {
            spawn_start_engine(actions);
            *mode = UiMode::Busy("starting engine…".to_string());
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let stacks = &actions.orchestrator.config.stacks;
            if stacks.len() <= 1 {
                // Only the built-in stack: no choice to make, start it —
                // exactly today's Ctrl+u.
                spawn_start_stack(actions, current_stack.to_string());
                *mode = UiMode::Busy(format!("starting stack {current_stack}…"));
            } else {
                let selected = stacks
                    .iter()
                    .position(|(n, _)| n == current_stack)
                    .unwrap_or(0);
                *mode = UiMode::StackPicker { selected };
            }
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Via start_all_managed (like CLI `start --all`), which pre-filters
            // to spawnable workers — naming the non-Rust ones directly would
            // bail at the first one, stranding the rest unstarted.
            spawn_start_all_managed(actions);
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

/// Name the surrounding terminal window / tmux pane after this instance, so
/// side-by-side instances are tellable apart from the window list without
/// looking inside. Re-applied whenever the branch changes.
fn set_terminal_title(branch: Option<&str>) {
    let title = match branch {
        Some(b) => format!("workers-dev ⎇ {b}"),
        None => "workers-dev".to_string(),
    };
    let _ = execute!(io::stdout(), SetTitle(title));
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

fn spawn_start_stack(actions: &Actions, name: String) {
    let orchestrator = actions.orchestrator.clone();
    spawn_action(actions, async move {
        orchestrator.start_stack(&name, false).await
    });
}

fn spawn_start_all_managed(actions: &Actions) {
    let orchestrator = actions.orchestrator.clone();
    spawn_action(actions, async move {
        orchestrator.start_all_managed(false).await
    });
}

fn spawn_start_engine(actions: &Actions) {
    let orchestrator = actions.orchestrator.clone();
    spawn_action(actions, async move {
        orchestrator.start_engine_quiet(ENGINE_START_WAIT_MS).await
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

fn spawn_toggle_ui_watch(actions: &Actions, worker: String) {
    let orchestrator = actions.orchestrator.clone();
    spawn_action(actions, async move {
        orchestrator.toggle_ui_watch(&worker).await.map(|_| ())
    });
}

/// Build the table rows, applying the name filter (case-insensitive). A group
/// header (with its post-filter worker count) is emitted only when that group
/// has at least one matching worker.
fn build_display_rows(views: &[WorkerView], filter: &str) -> Vec<DisplayRow> {
    let needle = filter.to_lowercase();
    let matches: Vec<usize> = views
        .iter()
        .enumerate()
        .filter(|(_, v)| needle.is_empty() || v.name.to_lowercase().contains(&needle))
        .map(|(idx, _)| idx)
        .collect();
    let mut rows = Vec::new();
    let mut last_group = None;
    for idx in matches.iter().copied() {
        let group = views[idx].group;
        if last_group != Some(group) {
            let count = matches.iter().filter(|&&i| views[i].group == group).count();
            rows.push(DisplayRow {
                kind: DisplayRowKind::Header(group, count),
            });
            last_group = Some(group);
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

/// Row index of the named worker in the display rows, if visible.
fn row_of_worker(display_rows: &[DisplayRow], views: &[WorkerView], name: &str) -> Option<usize> {
    display_rows
        .iter()
        .position(|r| matches!(r.kind, DisplayRowKind::Worker(idx) if views[idx].name == name))
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
        DisplayRowKind::Header(..) => None,
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
    // The header grows a line while the engine is down to carry the remedy.
    let header_h = if ctx.engine_error.is_some() { 4 } else { 3 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_h),
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
        draw_confirm_overlay(f, body, name, dependents, ctx);
    }
    if let UiMode::Busy(msg) = ctx.mode {
        draw_busy_overlay(f, body, msg, ctx);
    }
    if let UiMode::Deps {
        name,
        deps,
        dependents,
    } = ctx.mode
    {
        draw_deps_overlay(f, body, name, deps, dependents, ctx);
    }
    if matches!(ctx.mode, UiMode::Help) {
        draw_help_overlay(f, area, ctx.color_enabled);
    }
    if let UiMode::StackPicker { selected } = ctx.mode {
        stacks::draw_stack_picker_overlay(f, body, *selected, ctx);
    }
}

/// "   ● name …… status" with the table's live glyph and color for `name`.
/// Shared by the dependency and confirm-restart dialogs so every list of
/// workers in a dialog reads exactly like the worker table.
fn worker_status_line(ctx: &UiCtx, name: &str) -> Line<'static> {
    let (icon, status) = ctx
        .views
        .iter()
        .find(|v| v.name == name)
        .map(|v| {
            (
                status_icon(&v.display_status, ctx.spinner_frame),
                v.display_status.clone(),
            )
        })
        .unwrap_or(("○", "unknown".to_string()));
    let st = styled_if(ctx.color_enabled, status_style(&status));
    Line::from(vec![
        Span::styled(format!("   {icon} "), st),
        Span::raw(format!("{name:<24}")),
        Span::styled(status, st),
    ])
}

/// A blank spacer line (no visible glyphs). Trailing ones don't count as
/// content when measuring overflow, so a lone padding blank never triggers a
/// spurious "N more".
fn is_blank_line(line: &Line) -> bool {
    line.spans.iter().all(|s| s.content.trim().is_empty())
}

/// Render a centered dialog `width` wide inside `area`, titled and bordered.
/// `body` truncates when it doesn't fit — its tail is replaced by a single
/// "   … N more (resize to see all)" marker so a clipped dialog never looks
/// complete under an intact border. `pinned` lines (e.g. the y/n action row)
/// always render at the bottom and are never dropped; the truncation happens
/// only in the body above them.
fn draw_dialog(
    f: &mut Frame,
    area: Rect,
    title: String,
    mut body: Vec<Line<'static>>,
    pinned: Vec<Line<'static>>,
    width: u16,
    color: bool,
) {
    // Trailing blank spacers are padding, not content: trimming them keeps the
    // overflow test and the hidden-count honest (a lone trailing blank must not
    // push a dialog that otherwise fits into truncation).
    while body.last().is_some_and(is_blank_line) {
        body.pop();
    }

    let want_h = (body.len() + pinned.len()) as u16 + 2;
    let h = want_h.min(area.height.max(3));
    let inner = h.saturating_sub(2) as usize;
    // Pinned lines are reserved first; the body gets whatever height remains.
    let body_budget = inner.saturating_sub(pinned.len());

    let mut lines: Vec<Line> = if body.len() > body_budget {
        let keep = body_budget.saturating_sub(1);
        let hidden = body.len() - keep;
        let mut shown: Vec<Line> = body.into_iter().take(keep).collect();
        shown.push(Line::from(Span::styled(
            format!("   … {hidden} more (resize to see all)"),
            styled_if(color, hint_style()),
        )));
        shown
    } else {
        body
    };
    lines.extend(pinned);

    let popup = centered_rect_fixed(width, h, area);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(styled_if(color, overlay_bg_style())),
        ),
        popup,
    );
}

/// Dependency inspector: the selected worker's direct dependencies and
/// transitive dependents (the `r` blast radius), each with its live status —
/// so "can I start this?" and "what breaks if I bounce it?" are one keypress.
fn draw_deps_overlay(
    f: &mut Frame,
    area: Rect,
    name: &str,
    deps: &[String],
    dependents: &[String],
    ctx: &UiCtx,
) {
    let color = ctx.color_enabled;
    let section = |lines: &mut Vec<Line>, title: &str, workers: &[String]| {
        lines.push(Line::from(Span::styled(
            format!("   {title}"),
            styled_if(color, Style::default().add_modifier(Modifier::BOLD)),
        )));
        if workers.is_empty() {
            lines.push(Line::from(Span::styled(
                "   (none)",
                styled_if(color, hint_style()),
            )));
        }
        for w in workers {
            lines.push(worker_status_line(ctx, w));
        }
    };

    let mut lines = vec![Line::from("")];
    section(&mut lines, "depends on (direct)", deps);
    lines.push(Line::from(""));
    section(&mut lines, "needed by (restart blast radius)", dependents);
    lines.push(Line::from(""));

    draw_dialog(
        f,
        area,
        format!(" deps: {name} · any key to close "),
        lines,
        Vec::new(),
        58,
        color,
    );
}

fn draw_header(f: &mut Frame, area: Rect, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let mut spans = vec![Span::styled(
        "workers-dev",
        styled_if(color, header_accent_style()).add_modifier(Modifier::BOLD),
    )];

    // The branch sits right after the title — it's the one thing that tells
    // two side-by-side instances (worktrees, checkouts) apart, so it must
    // survive narrow terminals, which clip the header from the right.
    if let Some(branch) = ctx.repo_branch {
        let shown: String = if branch.chars().count() > BRANCH_MAX {
            let mut s: String = branch.chars().take(BRANCH_MAX - 1).collect();
            s.push('…');
            s
        } else {
            branch.to_string()
        };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("⎇ {shown}"),
            styled_if(color, branch_style()).add_modifier(Modifier::BOLD),
        ));
    }

    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        ctx.engine_url,
        styled_if(color, engine_url_style()),
    ));

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

    let mut lines = vec![Line::from(spans)];
    // Engine down: a bare "unreachable" is a dead end — say how to fix it.
    // Remedy before error detail, so right-edge clipping on a narrow terminal
    // eats the diagnostics rather than the fix.
    if let Some(err) = ctx.engine_error {
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "press e to start the engine (iii -c {})",
                    crate::orchestrator::ENGINE_CONFIG_REL
                ),
                styled_if(color, Style::default().fg(Color::Yellow)),
            ),
            Span::raw("  ·  "),
            Span::styled(err.to_string(), styled_if(color, muted_cell_style())),
        ]));
    }

    // No box title: the accent "workers-dev" span already names the pane.
    let header = Paragraph::new(lines).block(Block::default().borders(Borders::ALL));
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
            // No cells: a header row's real content is painted full-width,
            // below, after the table renders (Table has no colspan — a Cell
            // here would be clipped to the Worker column's width). `widths`
            // is non-empty so `column_count` (cell-count driven) never
            // affects layout; the `.header(...)` row alone still gives the
            // table 6 columns.
            DisplayRowKind::Header(..) => Row::new(Vec::<Cell>::new()),
            DisplayRowKind::Worker(idx) => {
                let v = &ctx.views[idx];
                let icon = status_icon(&v.display_status, ctx.spinner_frame);
                // Name cell is the mark (Space-toggled, for stack creation) +
                // glyph + name; unmarked rows get a space so names still line
                // up. The wide "(iii worker add)" label and the crash exit
                // code moved to the Process column, so the name sits right
                // next to its status instead of behind a column sized for the
                // longest label.
                let mark = if ctx.marked.contains(&v.name) {
                    "✓"
                } else {
                    " "
                };
                let name_cell = Cell::from(Span::styled(
                    format!("{mark}{icon} {}", v.name),
                    styled_if(color, status_style(&v.display_status)),
                ));
                let (process_text, process_st) = process_cell(v, color);
                let pid = v
                    .local_pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "—".to_string());
                let ui = crate::status::ui_watch_label(v.ui_watch);
                let ui_style = if v.ui_watch == Some(true) {
                    styled_if(color, Style::default().fg(Color::Cyan))
                } else {
                    styled_if(color, muted_cell_style())
                };
                Row::new(vec![
                    name_cell,
                    Cell::from(Span::styled(process_text, process_st)),
                    Cell::from(Span::styled(
                        v.engine_status.clone(),
                        styled_if(color, engine_style(&v.engine_status)),
                    )),
                    Cell::from(Span::styled(ui, ui_style)),
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

    // Title carries selection position ("Workers 3/48") so a long list is
    // navigable — you can tell where you are and how much is below the fold.
    let worker_count = |rows: &[DisplayRow]| {
        rows.iter()
            .filter(|r| matches!(r.kind, DisplayRowKind::Worker(_)))
            .count()
    };
    let total = worker_count(ctx.display_rows);
    let marked_suffix = match ctx.marked.len() {
        0 => String::new(),
        n => format!("· {n} marked "),
    };
    let title = match table_state.selected() {
        Some(sel) if sel < ctx.display_rows.len() => {
            format!(
                " Workers {}/{} {marked_suffix}",
                worker_count(&ctx.display_rows[..=sel]),
                total
            )
        }
        _ => format!(" Workers ({total}) {marked_suffix}"),
    };

    // Content-fit, left-packed widths: the worker name sits right next to its
    // status. Worker only has to fit a name now (label/exit moved to Process),
    // so it's tight; Uptime (Min) absorbs the right slack.
    let table = Table::new(
        rows,
        [
            Constraint::Length(24),
            Constraint::Length(10),
            Constraint::Length(11),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Min(8),
        ],
    )
    .header(
        Row::new(vec!["Worker", "Process", "Engine", "UI", "PID", "Uptime"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title(title))
    .row_highlight_style(styled_if(color, selection_row_style()));
    f.render_stateful_widget(table, area, table_state);

    // Header rows render no cells of their own (see the match arm above) —
    // paint their content here, spanning the table's full inner width
    // instead of being confined to one column. Assumes every row (header +
    // body) is its default 1-line height, true everywhere in this file.
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let rows_h = inner.height.saturating_sub(1) as usize; // minus the column-header line
    let offset = table_state.offset();
    for (i, row) in ctx.display_rows.iter().enumerate().skip(offset) {
        let y = i - offset;
        if y >= rows_h {
            break;
        }
        if let DisplayRowKind::Header(group, count) = row.kind {
            let label = format!(
                "── {} ({count}) ──",
                crate::status::group_label(group, ctx.current_stack)
            );
            let line = Line::from(Span::styled(
                label,
                styled_if(color, group_header_style(group)),
            ));
            f.render_widget(
                Paragraph::new(line),
                Rect::new(inner.x, inner.y + 1 + y as u16, inner.width, 1),
            );
        }
    }
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
            "(no output yet — press s to start this worker, or Ctrl+u to start the stack)"
        };
        vec![Line::from(Span::styled(
            msg,
            styled_if(color, hint_style()),
        ))]
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
    // Busy messages render as a centered dialog now, so the footer keeps its
    // help line in that mode.
    let (text, style) = match ctx.mode {
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

/// Dependents listed in the confirm dialog before it truncates; the deps
/// overlay (`d`) always shows the full list.
const CONFIRM_DEPENDENTS_SHOWN: usize = 8;

/// Confirm-restart dialog: the blast radius as a status-annotated list (a
/// comma-joined line clips for wide radii like llm-router's), so `y` is an
/// informed choice — a stopped dependent will be *started* by the restart,
/// and that's visible here instead of being a surprise.
fn draw_confirm_overlay(f: &mut Frame, area: Rect, name: &str, dependents: &[String], ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let mut lines = vec![Line::from("")];
    if dependents.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("   Restart {name}?"),
            styled_if(color, confirm_prompt_style()),
        )));
        lines.push(Line::from(Span::styled(
            "   no dependents — only this worker restarts",
            styled_if(color, hint_style()),
        )));
    } else {
        let n = dependents.len();
        let s = if n == 1 { "" } else { "s" };
        lines.push(Line::from(Span::styled(
            format!("   Restart {name} and {n} dependent{s}?"),
            styled_if(color, confirm_prompt_style()),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "   also restarts (stopped ones will start)",
            styled_if(color, Style::default().add_modifier(Modifier::BOLD)),
        )));
        for w in dependents.iter().take(CONFIRM_DEPENDENTS_SHOWN) {
            lines.push(worker_status_line(ctx, w));
        }
        if n > CONFIRM_DEPENDENTS_SHOWN {
            lines.push(Line::from(Span::styled(
                format!(
                    "   … and {} more (d lists the full blast radius)",
                    n - CONFIRM_DEPENDENTS_SHOWN
                ),
                styled_if(color, hint_style()),
            )));
        }
    }
    // The action row is pinned: it must survive even when the blast-radius
    // list is truncated on a short terminal — dropping it would leave a
    // blocking modal with no visible way to confirm or cancel.
    let pinned = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "   y/Enter ",
                styled_if(color, Style::default().fg(Color::Cyan)),
            ),
            Span::raw("restart      "),
            Span::styled("n/Esc ", styled_if(color, Style::default().fg(Color::Cyan))),
            Span::raw("cancel"),
        ]),
        Line::from(""),
    ];

    draw_dialog(
        f,
        area,
        " confirm restart ".to_string(),
        lines,
        pinned,
        58,
        color,
    );
}

fn draw_help_overlay(f: &mut Frame, area: Rect, color: bool) {
    let keys = [
        ("↑ ↓  k j", "select worker"),
        ("g G", "jump to first / last worker"),
        ("Space", "mark worker for a new stack"),
        ("s", "start selected worker"),
        ("x", "stop selected worker"),
        ("r", "restart selected + dependents"),
        ("w", "toggle injectable-UI watch (hot reload)"),
        ("d", "show dependencies + dependents"),
        ("f", "toggle live log follow"),
        ("PgUp PgDn", "scroll logs"),
        ("+ -", "resize the log pane"),
        ("/", "filter workers by name"),
        ("e", "start the iii engine"),
        ("Ctrl+u", "start stack (picker when several)"),
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
    lines.push(Line::from(Span::styled(
        "   UI: ui = ships injectable console UI · watch = hot-reload on (w)",
        styled_if(color, hint_style()),
    )));
    draw_dialog(
        f,
        area,
        " keys · any key to close ".to_string(),
        lines,
        Vec::new(),
        58,
        color,
    );
}

/// Center a fixed-size rect inside `r`, clamped to fit.
fn centered_rect_fixed(width: u16, height: u16, r: Rect) -> Rect {
    let w = width.min(r.width);
    let h = height.min(r.height);
    Rect::new(r.x + (r.width - w) / 2, r.y + (r.height - h) / 2, w, h)
}

/// Busy notice as a small centered dialog with an animated spinner. Input is
/// blocked while an action runs; a dialog makes the cause obvious where a
/// footer-only message was easy to miss.
fn draw_busy_overlay(f: &mut Frame, area: Rect, msg: &str, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let spin = SPINNER[ctx.spinner_frame % SPINNER.len()];
    let line = Line::from(vec![
        Span::styled(
            format!("  {spin} "),
            styled_if(color, Style::default().fg(Color::Yellow)),
        ),
        Span::raw(format!("{msg}  ")),
    ]);
    let w = (msg.chars().count() as u16).saturating_add(8).max(24);
    let popup = centered_rect_fixed(w, 3, area);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(vec![line]).block(
            Block::default()
                .borders(Borders::ALL)
                .style(styled_if(color, overlay_bg_style())),
        ),
        popup,
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Shared worker fixture for table-rendering tests: a real name, stable
    /// defaults for everything else that don't matter to the assertions.
    fn view_fixture(name: &str) -> WorkerView {
        WorkerView {
            name: name.to_string(),
            group: WorkerGroup::Other,
            spawnable: true,
            display_status: "stopped".to_string(),
            process_status: "stopped".to_string(),
            engine_status: "—".to_string(),
            local_pid: None,
            uptime: "—".to_string(),
            exit_code: None,
            ui_watch: None,
        }
    }

    /// Render just the worker table into an in-memory buffer, the way
    /// `draw_ui` would for one frame — for tests that only care about table
    /// content, not the full dashboard chrome. No filter; selection lands on
    /// the first worker row, same as a fresh dashboard.
    fn render_table_for_test(
        views: &[WorkerView],
        marked: &std::collections::HashSet<String>,
        width: u16,
        height: u16,
    ) -> ratatui::buffer::Buffer {
        let display_rows = build_display_rows(views, "");
        let mut table_state = TableState::default();
        table_state.select(first_worker_row(&display_rows));
        let ctx = UiCtx {
            engine_url: "",
            repo_branch: None,
            current_stack: "s",
            default_stack: "s",
            stacks: &[],
            views,
            marked,
            engine_error: None,
            display_rows: &display_rows,
            mode: &UiMode::Dashboard,
            filter: "",
            selected_name: None,
            log_lines: &[],
            log_scroll: 0,
            follow: true,
            log_height: LOG_HEIGHT_DEFAULT,
            table_width: TABLE_PANE_WIDTH,
            spinner_frame: 0,
            color_enabled: false,
            error: None,
        };
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| draw_table(f, f.area(), &mut table_state, &ctx))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// Flatten a rendered buffer into one string, so a test can assert on
    /// content without caring which exact cell it landed in.
    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// Regression test for a prior review finding: a group-header row used to
    /// be a single Cell sized to the Worker column (`Constraint::Length(24)`).
    /// Ratatui's `Table` has no colspan, so that clipped any label past 24
    /// chars — a real risk now that stack names are user-chosen. `draw_table`
    /// repaints visible header rows spanning the table's full width; this
    /// pins that the full label survives (not a 24-char prefix) AND keeps
    /// `group_header_style`'s color.
    #[test]
    fn group_header_row_spans_full_width_not_clipped() {
        let stack_name = "a-very-long-worktree-stack-name";
        let count = 999;
        let label = format!(
            "── {} ({count}) ──",
            crate::status::group_label(WorkerGroup::Stack, stack_name)
        );
        assert!(
            label.chars().count() > 24,
            "test label must exceed the Worker column width to exercise the bug"
        );

        let display_rows = vec![DisplayRow {
            kind: DisplayRowKind::Header(WorkerGroup::Stack, count),
        }];
        let marked = std::collections::HashSet::new();
        let ctx = UiCtx {
            engine_url: "",
            repo_branch: None,
            current_stack: stack_name,
            default_stack: stack_name,
            stacks: &[],
            views: &[],
            marked: &marked,
            engine_error: None,
            display_rows: &display_rows,
            mode: &UiMode::Dashboard,
            filter: "",
            selected_name: None,
            log_lines: &[],
            log_scroll: 0,
            follow: true,
            log_height: LOG_HEIGHT_DEFAULT,
            table_width: TABLE_PANE_WIDTH,
            spinner_frame: 0,
            color_enabled: true,
            error: None,
        };

        // TestBackend renders to an in-memory buffer — no real terminal
        // needed. 60x4 gives exactly: top border, column header, one data
        // row (our stack header), bottom border.
        let backend = ratatui::backend::TestBackend::new(60, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut table_state = TableState::default();
        terminal
            .draw(|f| draw_table(f, f.area(), &mut table_state, &ctx))
            .unwrap();

        let buf = terminal.backend().buffer();
        let row: String = (1..59).map(|x| buf[(x, 2)].symbol()).collect();
        assert_eq!(row.trim_end(), label, "header row was clipped: {row:?}");
        // group_header_style(Stack) = Style::default().fg(Cyan); with color
        // enabled the painted label must carry it, not just the raw text.
        assert_eq!(
            buf[(1, 2)].fg,
            Color::Cyan,
            "header row lost its group_header_style color"
        );
    }

    /// Companion to the above: the header-paint loop positions each visible
    /// header by its own offset-relative row, not just "the top of the
    /// window" — this pins that after the table has scrolled (offset > 0), a
    /// header that isn't display row 0 still lands on the correct screen
    /// line. Also `TestBackend`, still no real terminal.
    #[test]
    fn group_header_row_paints_correctly_after_scrolling() {
        let stack_name = "s";
        let views = vec![
            view_fixture("a1"),
            view_fixture("a2"),
            view_fixture("b1"),
            view_fixture("b2"),
        ];
        // Two groups, two headers: [H(Stack,2), W(a1), W(a2), H(Other,2), W(b1), W(b2)].
        let display_rows = vec![
            DisplayRow {
                kind: DisplayRowKind::Header(WorkerGroup::Stack, 2),
            },
            DisplayRow {
                kind: DisplayRowKind::Worker(0),
            },
            DisplayRow {
                kind: DisplayRowKind::Worker(1),
            },
            DisplayRow {
                kind: DisplayRowKind::Header(WorkerGroup::Other, 2),
            },
            DisplayRow {
                kind: DisplayRowKind::Worker(2),
            },
            DisplayRow {
                kind: DisplayRowKind::Worker(3),
            },
        ];
        let label_other = format!(
            "── {} (2) ──",
            crate::status::group_label(WorkerGroup::Other, stack_name)
        );
        let marked = std::collections::HashSet::new();
        let ctx = UiCtx {
            engine_url: "",
            repo_branch: None,
            current_stack: stack_name,
            default_stack: stack_name,
            stacks: &[],
            views: &views,
            marked: &marked,
            engine_error: None,
            display_rows: &display_rows,
            mode: &UiMode::Dashboard,
            filter: "",
            selected_name: None,
            log_lines: &[],
            log_scroll: 0,
            follow: true,
            log_height: LOG_HEIGHT_DEFAULT,
            table_width: TABLE_PANE_WIDTH,
            spinner_frame: 0,
            color_enabled: false,
            error: None,
        };

        // height=6: border + column header + 3 data rows + border. Selecting
        // the last row (display index 4, "b1") forces a scroll so display
        // rows 2..=4 are visible — landing the second header (display index
        // 3) on screen row 3, not row 2 (the "first data row" position the
        // un-scrolled test above pins).
        let backend = ratatui::backend::TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut table_state = TableState::default();
        table_state.select(Some(4));
        terminal
            .draw(|f| draw_table(f, f.area(), &mut table_state, &ctx))
            .unwrap();
        assert_eq!(
            table_state.offset(),
            2,
            "test setup must actually scroll (offset > 0) to be meaningful"
        );

        let buf = terminal.backend().buffer();
        let row: String = (1..39).map(|x| buf[(x, 3)].symbol()).collect();
        assert_eq!(
            row.trim_end(),
            label_other,
            "scrolled header row painted at the wrong screen line: {row:?}"
        );
    }

    #[test]
    fn marked_rows_show_a_check_and_the_title_counts_them() {
        let mut views = vec![view_fixture("alpha"), view_fixture("beta")];
        crate::status::assign_view_groups(
            &mut views,
            &["alpha".to_string(), "beta".to_string()]
                .into_iter()
                .collect(),
        );
        let marked: std::collections::HashSet<String> = ["beta".to_string()].into_iter().collect();
        let buf = render_table_for_test(&views, &marked, 40, 8);

        // Row-scoped, not a whole-buffer `contains` — a regression that
        // hardcoded the mark unconditionally (marking every row, or none)
        // would still pass a bare "a ✓ exists somewhere" check. Row layout
        // for this 40x8 backend: y=0 border, y=1 column header, y=2 the
        // "stack:s" group header (both workers are members), y=3 alpha
        // (unmarked), y=4 beta (marked).
        let alpha_row: String = (1..39).map(|x| buf[(x, 3)].symbol()).collect();
        let beta_row: String = (1..39).map(|x| buf[(x, 4)].symbol()).collect();
        assert!(
            beta_row.starts_with('✓'),
            "marked worker's row must start with a check:\n{beta_row:?}"
        );
        assert!(
            alpha_row.starts_with(' '),
            "unmarked worker's row must start with a space, not a check:\n{alpha_row:?}"
        );

        let text = buffer_text(&buf);
        assert!(text.contains("1 marked"), "title must count marks:\n{text}");
    }

    #[test]
    fn marked_worker_name_at_the_column_budget_is_not_truncated() {
        // Longest real worker name (provider-openai-codex, verified against
        // the */iii.worker.yaml directories) is 21 chars. Plus the 3-char
        // mark+icon+space prefix, that's exactly `Constraint::Length(24)` on
        // the Worker column — zero margin. Rendered at the table's real pane
        // width (TABLE_PANE_WIDTH), not a narrow test backend, so this
        // exercises the actual column budget rather than width pressure from
        // an undersized buffer.
        let name = "provider-openai-codex";
        assert_eq!(
            name.len(),
            21,
            "fixture drifted from the real worker name length"
        );
        let views = vec![view_fixture(name)];
        let marked: std::collections::HashSet<String> = [name.to_string()].into_iter().collect();
        let buf = render_table_for_test(&views, &marked, TABLE_PANE_WIDTH, 6);

        // y=0 border, y=1 column header, y=2 the single group header, y=3
        // the one worker row.
        let row: String = (1..TABLE_PANE_WIDTH - 1)
            .map(|x| buf[(x, 3)].symbol())
            .collect();
        assert!(
            row.contains(name),
            "21-char worker name must survive the Worker column untruncated:\n{row:?}"
        );
    }
}
