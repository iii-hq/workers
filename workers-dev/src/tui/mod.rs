//! The dashboard. One table over `compose::status`, one log pane, and the
//! lifecycle keys — every one of them a `compose::*` call, none of them a
//! local process.

mod theme;

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState};
use ratatui::{Frame, Terminal};

use crate::compose::{Compose, ContainerStatus, Progress};
use crate::config::LOG_TAIL_BYTES;
use crate::logs;

use theme::*;

const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];
const LOG_HEIGHT_MIN: u16 = 6;
const LOG_HEIGHT_DEFAULT: u16 = 18;
const MIN_TABLE_HEIGHT: u16 = 9;
const TABLE_PANE_WIDTH: u16 = 64;
const MIN_LOG_PANE_WIDTH: u16 = 36;
const TABLE_WIDTH_MIN: u16 = 44;
const PANE_GUTTER: u16 = 1;
const TWO_COL_MIN_WIDTH: u16 = TABLE_PANE_WIDTH + PANE_GUTTER + MIN_LOG_PANE_WIDTH;
const BRANCH_MAX: usize = 40;
/// Dependents listed before the confirm dialog truncates; `d` always shows all.
const CONFIRM_DEPENDENTS_SHOWN: usize = 8;

/// Button tracking plus SGR coordinates, and deliberately not crossterm's
/// `EnableMouseCapture`: that also sets `?1003h`, which reports every cell the
/// pointer crosses — one wake per pixel of travel — and, under tmux, flips
/// `mouse_any_flag`, taking drag-select, double-click-copy and middle-paste
/// away from the pane. Click and wheel arrive without any of that.
const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1006h";
const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1000l";

/// Always applicable, so always reserved: whatever the row is, these work.
const TAIL: &str = " Enter menu · f follow · / filter · ? keys · q quit ";

/// The row pinned above the containers. Its log is the compose daemon's own
/// output — the only place the startup tree, the adoption lines, the managed
/// engine's pid and every `error[CODE]` are ever printed.
const DAEMON_ROW: &str = "compose (daemon)";

enum UiMode {
    Dashboard,
    Filter,
    /// Browsing for a directory to offer workers from. The scanned entries live
    /// in the mode: labelling a directory costs a read_dir plus a stat per
    /// child, which is nothing once per navigation and a stutter per frame.
    Browse {
        dir: PathBuf,
        entries: Vec<BrowseEntry>,
        cursor: usize,
        filter: String,
    },
    Help,
    /// What the selected row can do, spelled out. No cursor: the keys are the
    /// interface and they keep working while it is open, so the menu teaches
    /// them instead of replacing them.
    Menu,
    Deps {
        name: String,
        deps: Vec<String>,
        dependents: Vec<String>,
    },
    ConfirmDown {
        name: String,
        dependents: Vec<String>,
    },
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModeKind {
    Dashboard,
    Filter,
    Browse,
    Help,
    Menu,
    Deps,
    Confirm,
    Quit,
}

fn mode_kind(mode: &UiMode) -> ModeKind {
    match mode {
        UiMode::Dashboard => ModeKind::Dashboard,
        UiMode::Filter => ModeKind::Filter,
        UiMode::Browse { .. } => ModeKind::Browse,
        UiMode::Help => ModeKind::Help,
        UiMode::Menu => ModeKind::Menu,
        UiMode::Deps { .. } => ModeKind::Deps,
        UiMode::ConfirmDown { .. } => ModeKind::Confirm,
        UiMode::Quit => ModeKind::Quit,
    }
}

/// A container the daemon reports, and the project that declares it.
#[derive(Clone)]
struct Entry {
    status: ContainerStatus,
    /// The compose file to address it through: the stack, or its own on-demand
    /// project.
    file: PathBuf,
    /// Started on demand from the repo list rather than declared by the stack.
    local: bool,
}

#[derive(Clone, Default)]
struct DashboardState {
    containers: Vec<Entry>,
    /// Snapshotted with the containers so a row index is stable for the frame,
    /// and so a directory added mid-session shows up on the next tick.
    repo_workers: Vec<crate::config::RepoWorker>,
    daemon_pid: u32,
    daemon_ours: bool,
    error: Option<String>,
    branch: Option<String>,
}

/// Row 0 is always the daemon. Then the stack the compose file declares, then
/// one group per directory workers are offered from — running or not, so
/// starting one is a keypress rather than a file edit.
///
/// Every row is selectable, headers included: a group header is where a
/// directory is dropped, and the stack header can start the project as
/// honestly as the daemon row can.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowRef {
    Daemon,
    StackHeader,
    /// Index into `repo_workers` of the first worker of a directory. That
    /// worker's `root` is the directory to label and to drop.
    Group(usize),
    Container(usize),
    Repo(usize),
}

struct Actions {
    compose: Arc<Compose>,
    in_flight: Arc<AtomicUsize>,
    errors: tokio::sync::mpsc::UnboundedSender<String>,
}

/// One directory in the browser, with what it holds. `offer` is how many of
/// its workers are not already on the list — the number that decides whether
/// Enter adds it or opens it.
#[derive(Clone)]
struct BrowseEntry {
    name: String,
    path: PathBuf,
    /// Workers found under it at all.
    total: usize,
    /// Of those, the ones no configured directory already provides.
    offer: usize,
    /// Already in the list: nothing to add, and `a` cannot add it twice.
    added: bool,
}

impl BrowseEntry {
    fn label(&self) -> String {
        match (self.added, self.total, self.offer) {
            (true, _, _) => "added".to_string(),
            (_, 0, _) => String::new(),
            (_, 1, 1) => "1 worker".to_string(),
            (_, total, offer) if total == offer => format!("{total} workers"),
            (_, total, offer) => format!("{offer} new of {total}"),
        }
    }
}

/// Where the two panes landed, so a click or a wheel can be aimed at one.
/// `Terminal::draw` discards its closure's return, so they come back by `&mut`.
#[derive(Clone, Copy, Default)]
struct Panes {
    table: Rect,
    log: Rect,
}

struct UiCtx<'a> {
    compose: &'a Compose,
    state: &'a DashboardState,
    progress: &'a Progress,
    rows: &'a [RowRef],
    mode: &'a UiMode,
    filter: &'a str,
    log_lines: &'a [String],
    log_title: &'a str,
    log_scroll: usize,
    follow: bool,
    log_height: u16,
    table_width: u16,
    /// What is running, for the footer. A modal would be worse: an action here
    /// is a detached call to a daemon that owns the lifecycle, and a cold
    /// container takes minutes — the keyboard has no business being held.
    busy: Option<&'a str>,
    selected: Option<RowRef>,
    spinner_frame: usize,
    color_enabled: bool,
    error: Option<&'a str>,
}

impl UiCtx<'_> {
    /// The state a row actually shows. While an operation is in flight the
    /// progress feed wins: `compose::status` cannot see inside its own
    /// operation and answers from the last durable snapshot.
    fn state_of(&self, container: &ContainerStatus) -> String {
        if self.progress.live() {
            if let Some(phase) = self.progress.phases.get(&container.container) {
                return normalize_phase(phase).to_string();
            }
            // No news about this one. `queued` only when the snapshot has
            // nothing better to say: the first events of an up fire before a
            // subscription can exist, so a container can be settled already.
            if container.state == "stopped" {
                return "queued".to_string();
            }
        }
        container.state.clone()
    }
}

/// compose's phases, in the words the table uses. Observed on the wire:
/// `waiting`, `starting`, `configuring`, `registering`, `ready`, `failed`.
fn normalize_phase(phase: &str) -> &'static str {
    let lower = phase.to_ascii_lowercase();
    if lower.contains("wait") || lower.contains("queued") || lower.contains("planned") {
        "queued"
    } else if lower.contains("fail") {
        "failed"
    } else if lower.contains("rolled back") || lower.contains("stopped") {
        "stopped"
    } else if lower.contains("ready") || lower.contains("already running") {
        "ready"
    } else {
        // starting, configuring, registering, resolving, installing — anything
        // compose reports while it is actively working on that container.
        "starting"
    }
}

pub async fn run(compose: Arc<Compose>) -> Result<()> {
    enable_raw_mode()?;
    // Push the terminal's title (XTWINOPS; ignored where unsupported) before
    // overwriting it, so quitting can restore it.
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        Print("\x1b[22;0t"),
        Print(MOUSE_ON)
    )?;

    let result: Result<()> = async {
        let backend = ratatui::backend::CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;

        compose.start_enabled_ui_children().await;

        let initial_branch = crate::git::current_branch(&compose.config.repo_root);
        set_terminal_title(initial_branch.as_deref());
        let mut state = snapshot(&compose, initial_branch).await;

        // Poll off the UI thread so a slow or unreachable engine cannot freeze
        // keyboard input.
        let (state_tx, mut state_rx) = tokio::sync::watch::channel(state.clone());
        let poll_interval = Duration::from_millis(crate::config::POLL_INTERVAL_MS);
        let poller = {
            let compose = compose.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(poll_interval).await;
                    let branch = crate::git::current_branch(&compose.config.repo_root);
                    if state_tx.send(snapshot(&compose, branch).await).is_err() {
                        break; // UI gone
                    }
                }
            })
        };

        let in_flight = Arc::new(AtomicUsize::new(0));
        // Detached actions report here: an eprintln would be lost under the
        // alternate screen.
        let (err_tx, mut err_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let actions = Actions {
            compose: compose.clone(),
            in_flight: in_flight.clone(),
            errors: err_tx,
        };

        let mut table_state = TableState::default();
        table_state.select(Some(0));
        let mut mode = UiMode::Dashboard;
        let mut filter = String::new();
        let mut follow = true;
        let mut log_scroll: usize = 0;
        let mut log_height: u16 = LOG_HEIGHT_DEFAULT;
        let mut table_width: u16 = TABLE_PANE_WIDTH;
        let mut spinner_frame: usize = 0;
        let mut busy_note: Option<String> = None;
        let mut error_banner: Option<String> = None;
        let mut running = true;
        let mut stop_on_exit = false;
        let mut needs_redraw = true;
        let mut last_busy_tick = Instant::now();
        let mut last_redraw = Instant::now();
        let mut log_lines: Vec<String> = Vec::new();
        let mut log_title = String::new();
        let mut last_log_read = Instant::now() - Duration::from_secs(60);
        let mut last_log_key = String::new();
        let mut panes = Panes::default();

        let color_enabled = compose.config.color_mode.enabled_for_tui();

        while running {
            let rows = build_rows(&state, &filter);
            clamp_selection(&mut table_state, &rows);
            let selected = table_state.selected().and_then(|i| rows.get(i).copied());
            let progress = compose.progress();
            let busy = progress.live() || in_flight.load(Ordering::SeqCst) > 0;

            // Re-read the log on selection change or on the poll cadence — the
            // active segment compose writes grows to 10 MiB, so this is not a
            // per-redraw cost.
            let key = log_key(&compose, &state, selected);
            // Only while following: the scroll window is measured back from the
            // end, so re-reading a growing file walks the line you stopped on
            // off the top of the pane.
            if key != last_log_key || (follow && last_log_read.elapsed() >= poll_interval) {
                let (title, path) = log_source(&compose, &state, selected);
                log_lines = path
                    .map(|p| logs::tail_file(&p, LOG_TAIL_BYTES))
                    .unwrap_or_default();
                log_title = title;
                last_log_key = key;
                last_log_read = Instant::now();
                needs_redraw = true;
            }

            if needs_redraw {
                if busy {
                    spinner_frame = spinner_frame.wrapping_add(1);
                }
                let ctx = UiCtx {
                    compose: &compose,
                    state: &state,
                    progress: &progress,
                    rows: &rows,
                    mode: &mode,
                    filter: &filter,
                    log_lines: &log_lines,
                    log_title: &log_title,
                    log_scroll,
                    follow,
                    log_height,
                    table_width,
                    busy: busy_note.as_deref(),
                    selected,
                    spinner_frame,
                    color_enabled,
                    error: error_banner.as_deref(),
                };
                terminal.draw(|f| panes = draw_ui(f, &mut table_state, &ctx))?;
                needs_redraw = false;
                last_redraw = Instant::now();
            }

            // Also the 1 Hz idle redraw and the spinner clock.
            if last_busy_tick.elapsed() >= poll_interval {
                last_busy_tick = Instant::now();
                if in_flight.load(Ordering::SeqCst) == 0 {
                    busy_note = None;
                }
                needs_redraw = true;
            }
            if busy && last_redraw.elapsed() >= Duration::from_millis(120) {
                needs_redraw = true;
            }

            if event::poll(Duration::from_millis(120))? {
                match event::read()? {
                    Event::Key(key) => {
                        needs_redraw = true;
                        error_banner = None; // any keypress acknowledges the banner
                        match mode_kind(&mode) {
                            ModeKind::Filter => handle_filter_key(key, &mut filter, &mut mode),
                            ModeKind::Browse => {
                                handle_browse_key(key, &mut mode, &mut busy_note, &state, &actions)
                            }
                            ModeKind::Help | ModeKind::Deps => mode = UiMode::Dashboard,
                            // The menu lists keys; the keys have to work while
                            // you read it, or it is a picture of an interface.
                            ModeKind::Menu => {
                                mode = UiMode::Dashboard;
                                if !matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
                                    handle_dashboard_key(
                                        key,
                                        &mut mode,
                                        &mut busy_note,
                                        progress.live(),
                                        panes.log.height.saturating_sub(3).max(1) as usize,
                                        &mut table_state,
                                        &rows,
                                        &state,
                                        selected,
                                        &mut follow,
                                        &mut log_scroll,
                                        &mut log_height,
                                        &mut table_width,
                                        &actions,
                                    );
                                }
                            }
                            ModeKind::Confirm => {
                                handle_confirm_key(key, &mut mode, &mut busy_note, &actions)
                            }
                            ModeKind::Quit => {
                                handle_quit_key(key, &mut mode, &mut running, &mut stop_on_exit)
                            }
                            ModeKind::Dashboard => handle_dashboard_key(
                                key,
                                &mut mode,
                                &mut busy_note,
                                progress.live(),
                                panes.log.height.saturating_sub(3).max(1) as usize,
                                &mut table_state,
                                &rows,
                                &state,
                                selected,
                                &mut follow,
                                &mut log_scroll,
                                &mut log_height,
                                &mut table_width,
                                &actions,
                            ),
                        }
                    }
                    // Only on the dashboard: a modal owns the screen, and the
                    // wheel over it was moving a selection nobody could see.
                    Event::Mouse(mouse) if mode_kind(&mode) == ModeKind::Dashboard => {
                        needs_redraw = true;
                        handle_mouse(
                            mouse,
                            panes,
                            &mut table_state,
                            &rows,
                            &mut follow,
                            &mut log_scroll,
                        );
                    }
                    Event::Resize(_, _) => needs_redraw = true,
                    _ => {}
                }
            }

            if let Ok(message) = err_rx.try_recv() {
                // Kept until a keypress acknowledges it. A failed mutation is
                // the only copy of a startup failure — a container that never
                // starts writes no record, so `compose::status` reports a plain
                // `stopped` with nothing to read.
                error_banner = Some(message);
                needs_redraw = true;
            }
            if state_rx.has_changed().unwrap_or(false) {
                state = state_rx.borrow_and_update().clone();
                needs_redraw = true;
            }
        }

        poller.abort();
        if stop_on_exit {
            compose.stop().await?;
        }
        Ok(())
    }
    .await;

    disable_raw_mode()?;
    execute!(
        io::stdout(),
        Print(MOUSE_OFF),
        Print("\x1b[23;0t"),
        LeaveAlternateScreen
    )?;
    result
}

async fn snapshot(compose: &Compose, branch: Option<String>) -> DashboardState {
    let daemon = compose.daemon_info().await;
    let daemon_ours = daemon.as_ref().map(|info| info.ours).unwrap_or(false);

    // `status` binds the stack (which `list` alone would not, on a daemon that
    // never loaded it); `list` then names every on-demand project in one call.
    let stack = compose.status().await;
    let (listed_pid, projects) = compose.projects().await.unwrap_or_default();

    let mut containers: Vec<Entry> = match &stack {
        Ok(status) => status
            .containers
            .iter()
            .cloned()
            .map(|status| Entry {
                status,
                file: compose.config.compose_path.clone(),
                local: false,
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    containers.extend(
        projects
            .into_iter()
            .filter(|project| project.file != compose.config.compose_path)
            .flat_map(|project| {
                project.containers.into_iter().map({
                    let file = project.file.clone();
                    move |status| Entry {
                        status,
                        file: file.clone(),
                        local: true,
                    }
                })
            }),
    );

    DashboardState {
        repo_workers: compose.repo_workers(),
        daemon_pid: match &stack {
            Ok(status) if status.daemon_pid > 0 => status.daemon_pid,
            _ => listed_pid.max(daemon.as_ref().map(|info| info.pid).unwrap_or_default()),
        },
        error: stack.err().map(|error| format!("{error:#}")),
        containers,
        daemon_ours,
        branch,
    }
}

/// The daemon row, the stack, then the repo — a repo worker that is running
/// shows as the container it became, so the two lists never disagree.
fn build_rows(state: &DashboardState, filter: &str) -> Vec<RowRef> {
    let needle = filter.to_ascii_lowercase();
    let matches = |name: &str| needle.is_empty() || name.to_ascii_lowercase().contains(&needle);

    let mut rows = vec![RowRef::Daemon];

    let stack: Vec<RowRef> = state
        .containers
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.local && matches(&entry.status.container))
        .map(|(index, _)| RowRef::Container(index))
        .collect();
    if !stack.is_empty() {
        rows.push(RowRef::StackHeader);
        rows.extend(stack);
    }

    // `discover_workers` keeps each directory's workers contiguous, so a header
    // is due whenever the root changes. Pushed lazily on the first row that
    // survives the filter, so a filter that empties a directory takes its
    // header with it.
    let mut last_root: Option<&Path> = None;
    for (index, worker) in state.repo_workers.iter().enumerate() {
        if !matches(&worker.name) {
            continue;
        }
        if last_root != Some(worker.root.as_path()) {
            rows.push(RowRef::Group(index));
            last_root = Some(worker.root.as_path());
        }
        match state
            .containers
            .iter()
            .position(|entry| entry.local && entry.status.container == worker.name)
        {
            Some(container) => rows.push(RowRef::Container(container)),
            None => rows.push(RowRef::Repo(index)),
        }
    }
    rows
}

/// The directory a group header stands for, and how to name it. The repo root
/// is `repo`; anything else is its own basename, which is what fits the column.
fn group_label(compose: &Compose, state: &DashboardState, index: usize) -> (PathBuf, String) {
    let Some(worker) = state.repo_workers.get(index) else {
        return (PathBuf::new(), "repo".to_string());
    };
    let root = worker.root.clone();
    if root == compose.config.repo_root {
        return (root, "repo".to_string());
    }
    let label = root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| root.display().to_string());
    (root, label)
}

/// The project that owns a row: the tracked stack, or the on-demand file.
fn project_of(compose: &Compose, state: &DashboardState, row: RowRef) -> PathBuf {
    match row {
        RowRef::Container(index) => state.containers[index].file.clone(),
        _ => compose.config.compose_path.clone(),
    }
}

/// What the selected row accepts, as `(key hint, what it does here)`.
///
/// The same rules `handle_dashboard_key` applies as match guards, written down
/// once and a keypress earlier — a guard that refuses is silent, and the row
/// selected at launch is the daemon row, where five of the keys are.
fn row_actions(
    is_container: bool,
    local: bool,
    watchable: bool,
    in_stack: bool,
    startable: bool,
) -> Vec<(&'static str, &'static str)> {
    if !is_container {
        // A repo worker, or the daemon row. The daemon has no lifecycle of its
        // own: the only thing to start from there is the project.
        return match (startable, in_stack) {
            (true, _) => vec![("s", "declare and start")],
            // Not a Rust binary. `s` would only print where to get it, so the
            // row says so instead of offering a key that refuses.
            (false, false) => Vec::new(),
            (false, true) => vec![("^u", "start the project")],
        };
    }

    let mut actions = vec![if local {
        ("s", "restart from the declaration")
    } else {
        ("s", "up")
    }];
    // Same word either way: on a stack row `x` opens a confirm that lists the
    // blast radius, so the footer does not need to carry the warning too.
    actions.push(("x", "stop"));
    // On a local row `r` is what `s` already does, minus re-reading the
    // declaration — one of them is enough.
    if !local {
        actions.push(("r", "restart just this"));
    }
    if watchable {
        actions.push(("w", "ui watch"));
    }
    // `d` reads `start_after` and dependents out of the stack file; off it,
    // both are empty by construction and the modal says `(none)` twice.
    if in_stack {
        actions.push(("d", "deps"));
    }
    actions
}

/// The five facts `row_actions` turns on, looked up for the current selection.
fn actions_for(
    compose: &Compose,
    state: &DashboardState,
    selected: Option<RowRef>,
) -> Vec<(&'static str, &'static str)> {
    let Some(row) = selected else {
        return Vec::new();
    };
    // A header is not a worker. It stands for a directory, and the things you
    // do to a directory are add another and drop this one.
    if let RowRef::Group(index) = row {
        let (root, _) = group_label(compose, state, index);
        return if root == compose.config.repo_root {
            vec![("a", "add a directory")]
        } else {
            vec![("x", "drop this directory"), ("a", "add a directory")]
        };
    }
    let name = selected_name(state, selected);
    let is_container = matches!(row, RowRef::Container(_));
    let local = matches!(row, RowRef::Container(index) if state.containers[index].local);
    row_actions(
        is_container,
        local,
        // What `toggle_ui_watch` itself refuses on, not the watch flag.
        name.as_deref()
            .is_some_and(|name| compose.ui_dir(name).is_some()),
        name.as_deref()
            .is_some_and(|name| compose.config.worker(name).is_some())
            || matches!(row, RowRef::Daemon | RowRef::StackHeader),
        matches!(row, RowRef::Repo(_))
            && name
                .as_deref()
                .and_then(|name| compose.repo_worker(name))
                .is_some_and(|worker| worker.bin.is_some()),
    )
}

/// The name a row acts on, whether it is running or only offered by the repo.
fn selected_name(state: &DashboardState, selected: Option<RowRef>) -> Option<String> {
    match selected? {
        RowRef::Container(index) => Some(state.containers.get(index)?.status.container.clone()),
        RowRef::Repo(index) => Some(state.repo_workers.get(index)?.name.clone()),
        _ => None,
    }
}

fn log_key(compose: &Compose, state: &DashboardState, selected: Option<RowRef>) -> String {
    let (title, path) = log_source(compose, state, selected);
    match path {
        Some(path) => format!("{title}\u{1}{}", path.display()),
        None => title,
    }
}

fn log_source(
    compose: &Compose,
    state: &DashboardState,
    selected: Option<RowRef>,
) -> (String, Option<PathBuf>) {
    match selected {
        Some(RowRef::Container(index)) => {
            let Some(entry) = state.containers.get(index) else {
                return (String::new(), None);
            };
            // The daemon hands us the path; each project logs to its own
            // directory and there is nothing to reconstruct.
            (
                entry.status.container.clone(),
                Some(entry.status.log_path.clone()),
            )
        }
        // Nothing has run, so there is nothing to tail — the empty pane says so.
        Some(RowRef::Repo(index)) => (
            state
                .repo_workers
                .get(index)
                .map(|worker| worker.name.clone())
                .unwrap_or_default(),
            None,
        ),
        _ => (
            DAEMON_ROW.to_string(),
            Some(compose.config.daemon_log_path.clone()),
        ),
    }
}

fn clamp_selection(table_state: &mut TableState, rows: &[RowRef]) {
    if rows.is_empty() {
        table_state.select(None);
        return;
    }
    table_state.select(Some(
        table_state.selected().unwrap_or(0).min(rows.len() - 1),
    ));
}

fn set_terminal_title(branch: Option<&str>) {
    let title = match branch {
        Some(branch) => format!("workers-dev ⎇ {branch}"),
        None => "workers-dev".to_string(),
    };
    let _ = execute!(io::stdout(), Print(format!("\x1b]2;{title}\x07")));
}

// ------------------------------------------------------------------- actions

fn spawn_action<F>(actions: &Actions, future: F)
where
    F: std::future::Future<Output = Result<()>> + Send + 'static,
{
    let in_flight = actions.in_flight.clone();
    let errors = actions.errors.clone();
    in_flight.fetch_add(1, Ordering::SeqCst);
    tokio::spawn(async move {
        if let Err(error) = future.await {
            let _ = errors.send(format!("{error:#}"));
        }
        in_flight.fetch_sub(1, Ordering::SeqCst);
    });
}

fn spawn_up(actions: &Actions, file: PathBuf, container: Option<String>) {
    let compose = actions.compose.clone();
    spawn_action(actions, async move {
        compose.up(&file, container.as_deref()).await
    });
}

fn spawn_down(actions: &Actions, file: PathBuf, container: String) {
    let compose = actions.compose.clone();
    spawn_action(
        actions,
        async move { compose.down(&file, &container).await },
    );
}

fn spawn_restart(actions: &Actions, file: PathBuf, worker: String) {
    let compose = actions.compose.clone();
    spawn_action(
        actions,
        async move { compose.restart(&file, &worker).await },
    );
}

fn spawn_toggle_ui_watch(actions: &Actions, file: PathBuf, worker: String) {
    let compose = actions.compose.clone();
    spawn_action(actions, async move {
        compose.toggle_ui_watch(&file, &worker).await
    });
}

fn start_on_demand(actions: &Actions, busy: &mut Option<String>, name: String) {
    match actions
        .compose
        .repo_worker(&name)
        .and_then(|worker| worker.bin.clone())
    {
        Some(bin) => {
            *busy = Some(format!("starting {name}…"));
            spawn_add_local(actions, name, bin);
        }
        // The banner, not the footer: the footer clears itself on the next
        // idle tick and this needs to be read.
        None => {
            let _ = actions.errors.send(format!(
                "{name} installs from the registry, not from this tree — \
                 `iii trigger compose::add worker={name}`"
            ));
        }
    }
}

/// Declare a repo worker in its own project and start it.
fn spawn_add_local(actions: &Actions, worker: String, bin: String) {
    let compose = actions.compose.clone();
    spawn_action(
        actions,
        async move { compose.add_local(&worker, &bin).await },
    );
}

fn spawn_cancel(actions: &Actions) {
    let compose = actions.compose.clone();
    spawn_action(actions, async move { compose.cancel().await });
}

// ----------------------------------------------------------------------- keys

#[allow(clippy::too_many_arguments)]
fn handle_dashboard_key(
    key: KeyEvent,
    mode: &mut UiMode,
    busy: &mut Option<String>,
    progress_live: bool,
    // One screenful of the log pane as it was last drawn, so a page keeps a
    // line or two of overlap instead of a fixed ten.
    log_page: usize,
    table_state: &mut TableState,
    rows: &[RowRef],
    state: &DashboardState,
    selected: Option<RowRef>,
    follow: &mut bool,
    log_scroll: &mut usize,
    log_height: &mut u16,
    table_width: &mut u16,
    actions: &Actions,
) {
    let compose = &actions.compose;
    let name = selected_name(state, selected);
    // Only a row backed by a running container has a project to address.
    let running = selected
        .filter(|row| matches!(row, RowRef::Container(_)))
        .map(|row| project_of(compose, state, row));
    let is_local =
        matches!(selected, Some(RowRef::Container(index)) if state.containers[index].local);
    match key.code {
        KeyCode::Char('q') => *mode = UiMode::Quit,
        KeyCode::Char('?') => *mode = UiMode::Help,
        KeyCode::Enter => *mode = UiMode::Menu,
        KeyCode::Char('/') => *mode = UiMode::Filter,
        KeyCode::Char('a') => *mode = open_browser(compose, state),
        // Only the daemon's own startup `--up` runs under an operation compose
        // registered; a restart this dashboard asks for passes an id compose
        // never registers, so there is nothing to cancel and Esc says nothing.
        KeyCode::Esc if progress_live => {
            *busy = Some("cancelling…".to_string());
            spawn_cancel(actions);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            move_selection(table_state, rows, false);
            *follow = true;
            *log_scroll = 0;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            move_selection(table_state, rows, true);
            *follow = true;
            *log_scroll = 0;
        }
        KeyCode::Home | KeyCode::Char('g') => {
            table_state.select(Some(0));
            clamp_selection(table_state, rows);
        }
        KeyCode::End | KeyCode::Char('G') => {
            table_state.select(Some(rows.len().saturating_sub(1)));
            clamp_selection(table_state, rows);
        }
        KeyCode::Char('f') => {
            *follow = !*follow;
            *log_scroll = 0;
        }
        KeyCode::PageUp => {
            *follow = false;
            *log_scroll += log_page;
        }
        KeyCode::PageDown => {
            *log_scroll = log_scroll.saturating_sub(log_page);
            if *log_scroll == 0 {
                *follow = true;
            }
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            *log_height = log_height.saturating_add(1);
            *table_width = table_width.saturating_sub(2).max(TABLE_WIDTH_MIN);
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            *log_height = log_height.saturating_sub(1).max(LOG_HEIGHT_MIN);
            *table_width = table_width.saturating_add(2).min(TABLE_PANE_WIDTH);
        }
        // The whole project, deliberately: a project-wide up rolls back
        // everything it started when one container fails, so it is never what
        // a single `s` does.
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            *busy = Some("starting the project…".to_string());
            spawn_up(actions, compose.config.compose_path.clone(), None);
        }
        KeyCode::Char('s') => match (name, running) {
            // An on-demand worker is re-declared on every start, so the
            // declaration follows the repo and the daemon re-reads it.
            (Some(name), Some(_)) if is_local => start_on_demand(actions, busy, name),
            (Some(name), Some(file)) => {
                *busy = Some(format!("up {name}…"));
                spawn_up(actions, file, Some(name));
            }
            // A repo worker nothing declares yet: declare it in its own
            // project and start it, which is the whole point of listing them.
            (Some(name), None) => start_on_demand(actions, busy, name),
            _ => {}
        },
        // Before the container arm: a group header drops its directory, which
        // is the only `x` that is not about a container.
        KeyCode::Char('x') if matches!(selected, Some(RowRef::Group(_))) => {
            let Some(RowRef::Group(index)) = selected else {
                return;
            };
            let (root, label) = group_label(compose, state, index);
            if root == compose.config.repo_root {
                return;
            }
            *busy = Some(format!("dropping {label}…"));
            let compose = actions.compose.clone();
            let errors = actions.errors.clone();
            spawn_action(actions, async move {
                let gone = compose.remove_worker_dir(&root)?;
                let plural = if gone == 1 { "" } else { "s" };
                let _ = errors.send(format!("✓ dropped {label} — {gone} worker{plural}"));
                Ok(())
            });
        }
        KeyCode::Char('x') => {
            if let (Some(name), Some(file)) = (name, running) {
                // An on-demand worker has no dependents to take down with it:
                // nothing in the stack can declare a dependency on it.
                if is_local {
                    *busy = Some(format!("down {name}…"));
                    spawn_down(actions, file, name);
                } else {
                    let dependents = compose.config.dependents(&name);
                    *mode = UiMode::ConfirmDown { name, dependents };
                }
            }
        }
        KeyCode::Char('r') => {
            if let (Some(name), Some(file)) = (name, running) {
                *busy = Some(format!("restarting {name}…"));
                spawn_restart(actions, file, name);
            }
        }
        KeyCode::Char('w') => {
            if let (Some(name), Some(file)) = (name, running) {
                *busy = Some(format!("ui watch {name}…"));
                spawn_toggle_ui_watch(actions, file, name);
            }
        }
        KeyCode::Char('d') => {
            if let Some(name) = name {
                let deps = compose
                    .config
                    .worker(&name)
                    .map(|worker| worker.deps.clone())
                    .unwrap_or_default();
                let dependents = compose.config.dependents(&name);
                *mode = UiMode::Deps {
                    name,
                    deps,
                    dependents,
                };
            }
        }
        _ => {}
    }
}

/// Click selects, the wheel scrolls whatever is under the pointer. Anything
/// else the terminal reports is the terminal's business, not ours.
fn handle_mouse(
    mouse: MouseEvent,
    panes: Panes,
    table_state: &mut TableState,
    rows: &[RowRef],
    follow: &mut bool,
    log_scroll: &mut usize,
) {
    let at = Position::new(mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::ScrollUp if panes.log.contains(at) => {
            *follow = false;
            *log_scroll += 3;
        }
        MouseEventKind::ScrollDown if panes.log.contains(at) => {
            *log_scroll = log_scroll.saturating_sub(3);
            if *log_scroll == 0 {
                *follow = true;
            }
        }
        MouseEventKind::ScrollUp => move_selection(table_state, rows, false),
        MouseEventKind::ScrollDown => move_selection(table_state, rows, true),
        MouseEventKind::Down(MouseButton::Left) if panes.table.contains(at) => {
            // The first body row sits below the border and the column header,
            // and ratatui writes the real first visible index back into
            // `offset` as it renders. Every row here is one line high; the day
            // one is not, this aims short.
            let first_row = panes.table.y + 2;
            if mouse.row < first_row {
                return;
            }
            let index = table_state.offset() + (mouse.row - first_row) as usize;
            if index < rows.len() {
                table_state.select(Some(index));
                *follow = true;
                *log_scroll = 0;
            }
        }
        _ => {}
    }
}

fn move_selection(table_state: &mut TableState, rows: &[RowRef], down: bool) {
    if rows.is_empty() {
        return;
    }
    let current = table_state.selected().unwrap_or(0);
    table_state.select(Some(if down {
        (current + 1).min(rows.len() - 1)
    } else {
        current.saturating_sub(1)
    }));
}

fn handle_filter_key(key: KeyEvent, filter: &mut String, mode: &mut UiMode) {
    match key.code {
        KeyCode::Enter => *mode = UiMode::Dashboard,
        KeyCode::Esc => {
            filter.clear();
            *mode = UiMode::Dashboard;
        }
        KeyCode::Backspace => {
            filter.pop();
        }
        KeyCode::Char(ch) => filter.push(ch),
        _ => {}
    }
}

/// Tab completes against the filesystem, because a path is the one thing here
/// nobody wants to type in full and nobody remembers exactly.
/// Label every visible subdirectory of `dir`.
///
/// Counted by the presence of a manifest, not by reading one: the authoritative
/// scan opens three files and parses YAML per worker, and the directory beside
/// this repo holds a dozen worktrees of it — some 850 workers, seconds of work
/// for a label. A directory's own name is the worker's name everywhere in this
/// tree, so the count is right where it matters, and the add itself still goes
/// through `discover_workers`, which is never wrong.
fn scan_dir(compose: &Compose, state: &DashboardState, dir: &Path) -> Vec<BrowseEntry> {
    let configured = compose.worker_dirs();
    let taken: HashSet<&str> = state
        .repo_workers
        .iter()
        .map(|worker| worker.name.as_str())
        .chain(
            compose
                .config
                .workers
                .iter()
                .map(|worker| worker.name.as_str()),
        )
        .collect();

    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<BrowseEntry> = read
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().to_string();
            // `.git`, `.workers-dev` and friends: never what you are looking for.
            if name.starts_with('.') {
                return None;
            }
            let names = worker_names_under(&path);
            Some(BrowseEntry {
                offer: names
                    .iter()
                    .filter(|name| !taken.contains(name.as_str()))
                    .count(),
                total: names.len(),
                added: path
                    .canonicalize()
                    .is_ok_and(|path| configured.contains(&path)),
                name,
                path,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// The workers a directory would offer, by name, at one stat each — the same
/// "it is one worker, or its children are" rule `discover_workers` applies.
fn worker_names_under(dir: &Path) -> Vec<String> {
    if dir.join("iii.worker.yaml").is_file() {
        return dir
            .file_name()
            .map(|name| vec![name.to_string_lossy().to_string()])
            .unwrap_or_default();
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    read.filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.join("iii.worker.yaml").is_file() {
                return None;
            }
            Some(path.file_name()?.to_string_lossy().to_string())
        })
        .collect()
}

/// Open the browser where the answer usually is: beside the repo, which is
/// where a sibling checkout like `harness-e2e` lives.
fn open_browser(compose: &Compose, state: &DashboardState) -> UiMode {
    let dir = compose
        .config
        .repo_root
        .parent()
        .unwrap_or(&compose.config.repo_root)
        .to_path_buf();
    UiMode::Browse {
        entries: scan_dir(compose, state, &dir),
        dir,
        cursor: 0,
        filter: String::new(),
    }
}

fn handle_browse_key(
    key: KeyEvent,
    mode: &mut UiMode,
    busy: &mut Option<String>,
    state: &DashboardState,
    actions: &Actions,
) {
    let UiMode::Browse {
        dir,
        entries,
        cursor,
        filter,
    } = mode
    else {
        return;
    };
    let visible: Vec<BrowseEntry> = entries
        .iter()
        .filter(|entry| {
            filter.is_empty()
                || entry
                    .name
                    .to_ascii_lowercase()
                    .contains(&filter.to_ascii_lowercase())
        })
        .cloned()
        .collect();
    let at = visible.get(*cursor).cloned();

    // Descend, rescanning where we land. The only navigation there is.
    let go = |mode: &mut UiMode, to: PathBuf| {
        *mode = UiMode::Browse {
            entries: scan_dir(&actions.compose, state, &to),
            dir: to,
            cursor: 0,
            filter: String::new(),
        };
    };

    match key.code {
        KeyCode::Esc => *mode = UiMode::Dashboard,
        KeyCode::Up | KeyCode::Char('\u{10}') => *cursor = cursor.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('\u{e}') => {
            *cursor = (*cursor + 1).min(visible.len().saturating_sub(1))
        }
        // Up a level, but only once the filter is spent — the convention every
        // fuzzy finder already taught.
        KeyCode::Backspace => {
            if filter.pop().is_none() {
                let parent = dir.parent().unwrap_or(dir).to_path_buf();
                go(mode, parent);
            } else {
                *cursor = 0;
            }
        }
        // Look inside, whatever the label says.
        KeyCode::Right => {
            if let Some(entry) = at {
                go(mode, entry.path);
            }
        }
        // One rule: a directory that offers something new is added, anything
        // else is opened. A worktree of this repo reads `0 new of 60` and
        // opens, which is the answer to the question it raises.
        KeyCode::Enter => {
            let Some(entry) = at else { return };
            if entry.offer > 0 && !entry.added {
                let path = entry.path.to_string_lossy().to_string();
                let label = entry.name.clone();
                let compose = actions.compose.clone();
                let errors = actions.errors.clone();
                *busy = Some(format!("adding {label}…"));
                *mode = UiMode::Dashboard;
                spawn_action(actions, async move {
                    let found = compose.add_worker_dir(&path)?;
                    let plural = if found == 1 { "" } else { "s" };
                    let _ = errors.send(format!("✓ {label} — {found} worker{plural} added"));
                    Ok(())
                });
            } else {
                go(mode, entry.path);
            }
        }
        KeyCode::Char(ch) => {
            filter.push(ch);
            *cursor = 0;
        }
        _ => {}
    }
}

fn handle_confirm_key(
    key: KeyEvent,
    mode: &mut UiMode,
    busy: &mut Option<String>,
    actions: &Actions,
) {
    let UiMode::ConfirmDown { name, .. } = mode else {
        return;
    };
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            let name = name.clone();
            spawn_down(
                actions,
                actions.compose.config.compose_path.clone(),
                name.clone(),
            );
            *busy = Some(format!("down {name}…"));
            *mode = UiMode::Dashboard;
        }
        KeyCode::Char('n') | KeyCode::Esc => *mode = UiMode::Dashboard,
        _ => {}
    }
}

/// Quitting is a choice, not a side effect: leaving the stack running is the
/// default, and tearing it down takes the managed engine with it.
///
/// The teardown is a flag rather than a spawn: a detached `compose::stop` races
/// the process exit that follows it, and the SDK drops a message sent after
/// shutdown without an error — so "stop everything" could silently stop nothing.
fn handle_quit_key(key: KeyEvent, mode: &mut UiMode, running: &mut bool, stop_on_exit: &mut bool) {
    match key.code {
        KeyCode::Char('l') | KeyCode::Enter => *running = false,
        KeyCode::Char('s') => {
            *stop_on_exit = true;
            *running = false;
        }
        KeyCode::Esc | KeyCode::Char('n') => *mode = UiMode::Dashboard,
        _ => {}
    }
}

// -------------------------------------------------------------------- drawing

fn styled_if(enabled: bool, style: Style) -> Style {
    if enabled {
        style
    } else {
        Style::default()
    }
}

fn draw_ui(f: &mut Frame, table_state: &mut TableState, ctx: &UiCtx) -> Panes {
    let area = f.area();
    // The header grows a line while something is wrong, to carry the remedy.
    let header_h = if ctx.state.error.is_some() { 4 } else { 3 };
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

    let panes;
    if body.width >= TWO_COL_MIN_WIDTH {
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
        panes = Panes {
            table: cols[0],
            log: cols[2],
        };
        draw_table(f, cols[0], table_state, ctx);
        draw_log_pane(f, cols[2], ctx);
    } else {
        let log_h = ctx
            .log_height
            .min(body.height.saturating_sub(MIN_TABLE_HEIGHT))
            .max(LOG_HEIGHT_MIN.min(body.height));
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(MIN_TABLE_HEIGHT), Constraint::Length(log_h)])
            .split(body);
        panes = Panes {
            table: rows[0],
            log: rows[1],
        };
        draw_table(f, rows[0], table_state, ctx);
        draw_log_pane(f, rows[1], ctx);
    }

    draw_footer(f, chunks[2], ctx);

    match ctx.mode {
        UiMode::ConfirmDown { name, dependents } => {
            draw_confirm_overlay(f, body, name, dependents, ctx)
        }
        UiMode::Deps {
            name,
            deps,
            dependents,
        } => draw_deps_overlay(f, body, name, deps, dependents, ctx),
        UiMode::Help => draw_help_overlay(f, area, ctx.color_enabled),
        UiMode::Menu => draw_menu_overlay(f, body, ctx),
        UiMode::Browse {
            dir,
            entries,
            cursor,
            filter,
        } => draw_browse_overlay(f, body, dir, entries, *cursor, filter, ctx),
        UiMode::Quit => draw_quit_overlay(f, body, ctx),
        _ => {}
    }
    panes
}

fn draw_header(f: &mut Frame, area: Rect, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let mut spans = vec![Span::styled(
        "workers-dev",
        styled_if(color, header_accent_style()),
    )];

    // The branch is the one thing that tells two side-by-side worktree
    // instances apart, so it sits early: narrow terminals clip from the right.
    if let Some(branch) = ctx.state.branch.as_deref() {
        let shown: String = if branch.chars().count() > BRANCH_MAX {
            branch.chars().take(BRANCH_MAX - 1).chain(['…']).collect()
        } else {
            branch.to_string()
        };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("⎇ {shown}"),
            styled_if(color, branch_style()).add_modifier(Modifier::BOLD),
        ));
    }

    if ctx.state.error.is_some() {
        spans.push(Span::styled(
            "  ⚠ unreachable",
            styled_if(color, Style::default().fg(Color::Red)).add_modifier(Modifier::BOLD),
        ));
    } else {
        let (mut ready, mut failed, mut other) = (0u32, 0u32, 0u32);
        for entry in &ctx.state.containers {
            match ctx.state_of(&entry.status).as_str() {
                "ready" => ready += 1,
                "failed" => failed += 1,
                _ => other += 1,
            }
        }
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            format!("●{ready}"),
            styled_if(color, Style::default().fg(Color::Green)),
        ));
        if failed > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("✗{failed}"),
                styled_if(color, Style::default().fg(Color::Red)),
            ));
        }
        if other > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("○{other}"),
                styled_if(color, muted_cell_style()),
            ));
        }
    }

    if ctx.progress.live() {
        let done = ctx
            .progress
            .phases
            .values()
            .filter(|phase| normalize_phase(phase) == "ready")
            .count();
        spans.push(Span::styled(
            format!("   up {done}/{}", ctx.compose.config.workers.len()),
            styled_if(color, Style::default().fg(Color::Yellow)),
        ));
    }
    if ctx.state.daemon_pid > 0 {
        let ownership = if ctx.state.daemon_ours {
            "managed"
        } else {
            "attached"
        };
        spans.push(Span::styled(
            format!("   daemon:{} {ownership}", ctx.state.daemon_pid),
            styled_if(color, muted_cell_style()),
        ));
    }
    if !ctx.filter.is_empty() {
        spans.push(Span::styled(
            format!("   filter:{}", ctx.filter),
            styled_if(color, Style::default().fg(Color::Yellow)),
        ));
    }

    // Constants last. A narrow terminal clips the header from the right, and
    // the engine URL never changes while the counts and the filter do — with
    // them ahead of it an applied filter needed ~150 columns to be visible.
    spans.push(Span::raw("   "));
    spans.push(Span::styled(
        &ctx.compose.config.engine_url,
        styled_if(color, engine_url_style()),
    ));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("ns:{}", ctx.compose.config.namespace),
        styled_if(color, muted_cell_style()),
    ));

    let mut lines = vec![Line::from(spans)];
    // A bare error is a dead end. The remedy comes first so right-edge
    // clipping eats the diagnostics, not the fix.
    if let Some(error) = ctx.state.error.as_deref() {
        lines.push(Line::from(vec![
            Span::styled(
                remedy_for(error, ctx).to_string(),
                styled_if(color, Style::default().fg(Color::Yellow)),
            ),
            Span::raw("  ·  "),
            Span::styled(error.to_string(), styled_if(color, muted_cell_style())),
        ]));
    }

    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

/// Every compose error code names its own fix; only the two structural ones
/// (no engine, no daemon) need us to say it.
fn remedy_for(error: &str, ctx: &UiCtx) -> &'static str {
    if error.contains("function_not_found") {
        "no compose daemon in this namespace — quit and relaunch to start one"
    } else if error.contains("Connection") || error.contains("connect") || error.contains("closed")
    {
        "engine unreachable — quit and relaunch to start the stack"
    } else if ctx.state.daemon_pid == 0 {
        "no compose daemon yet — see the daemon row's log"
    } else {
        "see the compose (daemon) row for the daemon's own output"
    }
}

fn draw_table(f: &mut Frame, area: Rect, table_state: &mut TableState, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let rows: Vec<Row> = ctx
        .rows
        .iter()
        .map(|row| match row {
            RowRef::Daemon => {
                let (state, style) = if ctx.state.daemon_pid > 0 {
                    ("serving".to_string(), Style::default().fg(Color::Cyan))
                } else {
                    ("—".to_string(), muted_cell_style())
                };
                Row::new(vec![
                    Cell::from(Span::styled(
                        format!(" ▣ {DAEMON_ROW}"),
                        styled_if(color, Style::default().fg(Color::Cyan)),
                    )),
                    Cell::from(Span::styled(state, styled_if(color, style))),
                    Cell::from(Span::styled(
                        pid_cell(Some(ctx.state.daemon_pid).filter(|pid| *pid > 0)),
                        styled_if(color, muted_cell_style()),
                    )),
                    Cell::from(Span::styled("—", styled_if(color, muted_cell_style()))),
                    Cell::from(Span::raw("")),
                ])
            }
            RowRef::StackHeader => group_row(ctx, "stack", RowRef::StackHeader, color),
            RowRef::Group(index) => {
                let (_, label) = group_label(ctx.compose, ctx.state, *index);
                group_row(ctx, &label, RowRef::Group(*index), color)
            }
            RowRef::Container(index) => {
                let entry = &ctx.state.containers[*index];
                let state = ctx.state_of(&entry.status);
                let icon = status_icon(&state, ctx.spinner_frame);
                let style = styled_if(color, status_style(&state));
                let (ui, ui_style) = ui_cell(ctx, &entry.status.container, color);
                Row::new(vec![
                    Cell::from(Span::styled(
                        format!(" {icon} {}", entry.status.container),
                        style,
                    )),
                    Cell::from(Span::styled(state, style)),
                    Cell::from(Span::styled(
                        pid_cell(entry.status.pid),
                        styled_if(color, muted_cell_style()),
                    )),
                    Cell::from(Span::styled(ui, ui_style)),
                    Cell::from(Span::styled(
                        short_error(entry.status.last_error.as_deref()),
                        styled_if(color, Style::default().fg(Color::Red)),
                    )),
                ])
            }
            // Declared by nothing yet: `s` is what turns it into a container.
            RowRef::Repo(index) => {
                let worker = &ctx.state.repo_workers[*index];
                let muted = styled_if(color, muted_cell_style());
                let (state, state_style) = match worker.bin {
                    Some(_) => ("—", muted),
                    None => ("registry", muted),
                };
                let (ui, ui_style) = ui_cell(ctx, &worker.name, color);
                Row::new(vec![
                    Cell::from(Span::styled(format!("   {}", worker.name), muted)),
                    Cell::from(Span::styled(state, state_style)),
                    Cell::from(Span::styled("—", muted)),
                    Cell::from(Span::styled(ui, ui_style)),
                    Cell::from(Span::raw("")),
                ])
            }
        })
        .collect();

    // Group headers are labels; neither the count nor the position counts them.
    let is_worker = |row: &RowRef| matches!(row, RowRef::Container(_) | RowRef::Repo(_));
    let total = ctx.rows.iter().filter(|row| is_worker(row)).count();
    let title = match table_state.selected() {
        Some(selected) if ctx.rows.get(selected).is_some_and(is_worker) => {
            let position = ctx.rows[..=selected]
                .iter()
                .filter(|row| is_worker(row))
                .count();
            format!(" Workers {position}/{total} ")
        }
        _ => format!(" Workers ({total}) "),
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(24),
            Constraint::Length(9),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new(vec!["Worker", "State", "PID", "UI", "Last error"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title(title))
    .row_highlight_style(styled_if(color, selection_row_style()));

    // ratatui only ever scrolls the offset *up to* the selection, never back
    // past it, so a filter or a shrinking list can strand the offset and hide
    // the pinned daemon row for good. Clamp it to what the pane can show.
    let visible = area.height.saturating_sub(3).max(1) as usize; // borders + column header
    let max_offset = ctx.rows.len().saturating_sub(visible);
    *table_state.offset_mut() = (*table_state.offset_mut()).min(max_offset);
    f.render_stateful_widget(table, area, table_state);
}

/// A group divider. Table has no colspan, so the label lives in the first
/// column — it fits, and the alternative is repainting rows by hand.
fn group_row(ctx: &UiCtx, label: &str, header: RowRef, color: bool) -> Row<'static> {
    let count = ctx
        .rows
        .iter()
        .skip_while(|row| **row != header)
        .skip(1)
        .take_while(|row| !matches!(row, RowRef::StackHeader | RowRef::Group(_)))
        .count();
    Row::new(vec![Cell::from(Span::styled(
        format!("── {label} ({count}) ──"),
        styled_if(color, hint_style()),
    ))])
}

/// compose writes five fixed phrases here; the column is ~10 columns wide and
/// the exit code is the only part that differs between two crashes.
fn short_error(error: Option<&str>) -> String {
    let Some(error) = error else {
        return String::new();
    };
    match error.strip_prefix("exited unexpectedly with ") {
        Some(code) => format!("exit {code}"),
        None => error.to_string(),
    }
}

fn pid_cell(pid: Option<u32>) -> String {
    pid.map(|pid| pid.to_string()).unwrap_or_else(|| "—".into())
}

fn ui_cell(ctx: &UiCtx, container: &str, color: bool) -> (String, Style) {
    match ctx.compose.ui_watch(container) {
        Some(true) => (
            "watch".to_string(),
            styled_if(color, Style::default().fg(Color::Cyan)),
        ),
        Some(false) => ("ui".to_string(), styled_if(color, muted_cell_style())),
        None => ("—".to_string(), styled_if(color, muted_cell_style())),
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
        vec![Line::from(Span::styled(
            "(no output yet)",
            styled_if(color, hint_style()),
        ))]
    } else {
        ctx.log_lines[start..end]
            .iter()
            .map(|line| logs::log_line_to_ratatui(line, inner_width, color))
            .collect()
    };
    lines.truncate(inner_height.max(1));

    let mut title = vec![
        Span::raw(" logs: "),
        Span::styled(
            ctx.log_title.to_string(),
            styled_if(color, log_title_style()),
        ),
        Span::raw("  "),
    ];
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

    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Line::from(title)),
        ),
        area,
    );
}

fn draw_footer(f: &mut Frame, area: Rect, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    if let Some(notice) = ctx.error {
        // The same channel carries both, because both have to survive until
        // they are read. A message that brought its own glyph is good news.
        let (text, style) = match notice.starts_with('✓') {
            true => (format!(" {notice} "), Style::default().fg(Color::Green)),
            false => (format!(" ⚠ {notice} "), Style::default().fg(Color::Red)),
        };
        f.render_widget(
            Paragraph::new(text).style(styled_if(color, style).add_modifier(Modifier::BOLD)),
            area,
        );
        return;
    }
    // Ahead of the help line: what is running matters more than what the keys
    // are, and it is the only thing left saying an action is in flight.
    if let Some(message) = ctx.busy {
        let spin = SPINNER[ctx.spinner_frame % SPINNER.len()];
        f.render_widget(
            Paragraph::new(format!(" {spin} {message} "))
                .style(styled_if(color, Style::default().fg(Color::Yellow))),
            area,
        );
        return;
    }
    let (text, style) = match ctx.mode {
        UiMode::Filter => (
            format!(" filter: {}_   (Enter apply · Esc clear) ", ctx.filter),
            styled_if(color, Style::default().fg(Color::Yellow)),
        ),

        // What this row accepts, not a fixed string: three of the five row
        // kinds refuse most of the keys, and a guard that refuses is silent.
        _ => return draw_row_help(f, area, ctx),
    };
    f.render_widget(Paragraph::new(text).style(style), area);
}

/// Keys for the selected row, then the ones that always work. The tail is
/// reserved first, so narrowing drops row actions — which the `Enter` menu
/// still lists — rather than the way out.
fn draw_row_help(f: &mut Frame, area: Rect, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let mut spans = Vec::new();
    let mut used = TAIL.chars().count();
    for (key, label) in actions_for(ctx.compose, ctx.state, ctx.selected) {
        let width = key.chars().count() + label.chars().count() + 4;
        if used + width > area.width as usize {
            break;
        }
        used += width;
        spans.push(Span::styled(
            format!(" {key} "),
            styled_if(color, Style::default().fg(Color::Cyan)),
        ));
        spans.push(Span::styled(label, styled_if(color, footer_style())));
        spans.push(Span::raw(" ·"));
    }
    spans.push(Span::styled(TAIL, styled_if(color, footer_style())));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn is_blank_line(line: &Line) -> bool {
    line.spans.iter().all(|s| s.content.trim().is_empty())
}

/// A centered dialog. The body truncates with an honest marker; `pinned` lines
/// (the action row) always render, so a blocking modal never loses its way out.
fn draw_dialog(
    f: &mut Frame,
    area: Rect,
    title: String,
    mut body: Vec<Line<'static>>,
    pinned: Vec<Line<'static>>,
    width: u16,
    color: bool,
) {
    while body.last().is_some_and(is_blank_line) {
        body.pop();
    }

    let want_h = (body.len() + pinned.len()) as u16 + 2;
    let height = want_h.min(area.height.max(3));
    let inner = height.saturating_sub(2) as usize;
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

    let popup = centered_rect_fixed(width, height, area);
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

/// "   ● name …… state", with the table's own glyph and color, so every list
/// of containers in a dialog reads like the table.
fn container_status_line(ctx: &UiCtx, name: &str) -> Line<'static> {
    let state = ctx
        .state
        .containers
        .iter()
        .find(|entry| entry.status.container == name)
        .map(|entry| ctx.state_of(&entry.status))
        .unwrap_or_else(|| "—".to_string());
    let style = styled_if(ctx.color_enabled, status_style(&state));
    Line::from(vec![
        Span::styled(
            format!("   {} ", status_icon(&state, ctx.spinner_frame)),
            style,
        ),
        Span::raw(format!("{name:<24}")),
        Span::styled(state, style),
    ])
}

fn draw_deps_overlay(
    f: &mut Frame,
    area: Rect,
    name: &str,
    deps: &[String],
    dependents: &[String],
    ctx: &UiCtx,
) {
    let color = ctx.color_enabled;
    let section = |lines: &mut Vec<Line>, title: &str, names: &[String]| {
        lines.push(Line::from(Span::styled(
            format!("   {title}"),
            styled_if(color, Style::default().add_modifier(Modifier::BOLD)),
        )));
        if names.is_empty() {
            lines.push(Line::from(Span::styled(
                "   (none)",
                styled_if(color, hint_style()),
            )));
        }
        for name in names {
            lines.push(container_status_line(ctx, name));
        }
    };

    let mut lines = vec![Line::from("")];
    section(&mut lines, "start_after (direct)", deps);
    lines.push(Line::from(""));
    section(&mut lines, "needed by (stop blast radius)", dependents);
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

/// `compose::down` takes the transitive dependents with it, dependents first.
/// Showing them is what makes `y` an informed answer.
fn draw_confirm_overlay(f: &mut Frame, area: Rect, name: &str, dependents: &[String], ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let mut lines = vec![Line::from("")];
    if dependents.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("   Stop {name}?"),
            styled_if(color, confirm_prompt_style()),
        )));
        lines.push(Line::from(Span::styled(
            "   nothing depends on it — only this container stops",
            styled_if(color, hint_style()),
        )));
    } else {
        let count = dependents.len();
        let plural = if count == 1 { "" } else { "s" };
        lines.push(Line::from(Span::styled(
            format!("   Stop {name} and {count} dependent{plural}?"),
            styled_if(color, confirm_prompt_style()),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "   also stops, dependents first",
            styled_if(color, Style::default().add_modifier(Modifier::BOLD)),
        )));
        for name in dependents.iter().take(CONFIRM_DEPENDENTS_SHOWN) {
            lines.push(container_status_line(ctx, name));
        }
        if count > CONFIRM_DEPENDENTS_SHOWN {
            lines.push(Line::from(Span::styled(
                format!(
                    "   … and {} more (d lists the full blast radius)",
                    count - CONFIRM_DEPENDENTS_SHOWN
                ),
                styled_if(color, hint_style()),
            )));
        }
    }
    let pinned = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "   y/Enter ",
                styled_if(color, Style::default().fg(Color::Cyan)),
            ),
            Span::raw("stop      "),
            Span::styled("n/Esc ", styled_if(color, Style::default().fg(Color::Cyan))),
            Span::raw("cancel"),
        ]),
        Line::from(""),
    ];
    draw_dialog(
        f,
        area,
        " confirm stop ".to_string(),
        lines,
        pinned,
        58,
        color,
    );
}

/// Leaving the stack up is the default: the daemon is not our child to kill,
/// and `compose::stop` also stops the engine when compose owns it.
fn draw_quit_overlay(f: &mut Frame, area: Rect, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "   Quit workers-dev",
            styled_if(color, confirm_prompt_style()),
        )),
        Line::from(""),
    ];
    let pinned = vec![
        Line::from(vec![
            Span::styled(
                "   l/Enter ",
                styled_if(color, Style::default().fg(Color::Cyan)),
            ),
            Span::raw("leave the stack running"),
        ]),
        Line::from(vec![
            Span::styled(
                "   s       ",
                styled_if(color, Style::default().fg(Color::Cyan)),
            ),
            Span::raw("stop everything (compose::stop)"),
        ]),
        Line::from(vec![
            Span::styled(
                "   Esc     ",
                styled_if(color, Style::default().fg(Color::Cyan)),
            ),
            Span::raw("cancel"),
        ]),
        Line::from(""),
    ];
    draw_dialog(f, area, " quit ".to_string(), lines, pinned, 62, color);
}

/// The directory browser. `draw_dialog` cannot scroll — it truncates from the
/// bottom — so the window is computed here, with the same arithmetic, and the
/// cursor rides the bottom edge the way a fuzzy finder does.
#[allow(clippy::too_many_arguments)]
/// Wide enough for a name column plus a label, and no wider: the dialog is
/// centred over the table and a reader still needs the row behind it.
const WIDTH: u16 = 62;

fn draw_browse_overlay(
    f: &mut Frame,
    area: Rect,
    dir: &Path,
    entries: &[BrowseEntry],
    cursor: usize,
    filter: &str,
    ctx: &UiCtx,
) {
    let color = ctx.color_enabled;
    let needle = filter.to_ascii_lowercase();
    let visible: Vec<&BrowseEntry> = entries
        .iter()
        .filter(|entry| needle.is_empty() || entry.name.to_ascii_lowercase().contains(&needle))
        .collect();

    let pinned = vec![Line::from(vec![
        Span::styled(
            format!("   {filter}_"),
            styled_if(color, Style::default().fg(Color::Yellow)),
        ),
        Span::styled(
            "   Enter add or open · → look inside · ⌫ up · Esc",
            styled_if(color, hint_style()),
        ),
    ])];

    let budget = (area.height.max(3) as usize)
        .saturating_sub(2 + pinned.len())
        .max(1);
    let start = browse_window(cursor, visible.len(), budget);

    let mut body: Vec<Line> = visible
        .iter()
        .enumerate()
        .skip(start)
        .take(budget)
        .map(|(index, entry)| {
            let label = entry.label();
            let line = Line::from(vec![
                Span::raw(format!("  {:<26}", entry.name)),
                Span::styled(
                    label.clone(),
                    styled_if(
                        color,
                        // Only what can be added is worth the eye: everything
                        // else on this screen is a place to go, not a choice.
                        if entry.offer > 0 && !entry.added {
                            Style::default().fg(Color::Green)
                        } else {
                            hint_style()
                        },
                    ),
                ),
            ]);
            if index == cursor {
                line.style(styled_if(color, selection_row_style()))
            } else {
                line
            }
        })
        .collect();
    if body.is_empty() {
        body.push(Line::from(Span::styled(
            "   (nothing here)",
            styled_if(color, hint_style()),
        )));
    }

    // The tail of a path is where you are; the head is how you got there. A
    // long one loses its head rather than the counter.
    let counter = match visible.len() {
        0 => "   (empty)".to_string(),
        len => format!("   {}/{len}", cursor + 1),
    };
    let room = (WIDTH as usize).saturating_sub(counter.chars().count() + 4);
    let shown = shorten_home(dir);
    let path = match shown.chars().count() > room {
        true => format!(
            "…{}",
            shown
                .chars()
                .skip(shown.chars().count() - room + 1)
                .collect::<String>()
        ),
        false => shown,
    };
    draw_dialog(
        f,
        area,
        format!(" {path}{counter} "),
        body,
        pinned,
        WIDTH,
        color,
    );
}

/// The first row to draw. `draw_dialog` truncates from the bottom and cannot
/// scroll, so the window is chosen here with its own arithmetic: the cursor
/// rides the bottom edge on the way down and the top edge on the way up, the
/// way a fuzzy finder does.
fn browse_window(cursor: usize, len: usize, budget: usize) -> usize {
    if len <= budget {
        return 0;
    }
    cursor
        .saturating_sub(budget.saturating_sub(1))
        .min(len - budget)
}

/// `~` is what a person recognises; the absolute path is noise in a title.
fn shorten_home(dir: &Path) -> String {
    let shown = dir.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && shown.starts_with(&home) => shown.replacen(&home, "~", 1),
        _ => shown,
    }
}

/// The applicable subset of `?`, named for the row it is about. A row that
/// accepts nothing says so — that is the answer the dashboard never gave.
fn draw_menu_overlay(f: &mut Frame, area: Rect, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let name = selected_name(ctx.state, ctx.selected).unwrap_or_else(|| DAEMON_ROW.to_string());
    let actions = actions_for(ctx.compose, ctx.state, ctx.selected);

    let mut lines = vec![Line::from("")];
    if actions.is_empty() {
        lines.push(Line::from(Span::styled(
            "   nothing to run from here",
            styled_if(color, hint_style()),
        )));
        lines.push(Line::from(Span::styled(
            "   it installs from the registry, not from this tree:",
            styled_if(color, hint_style()),
        )));
        lines.push(Line::from(Span::styled(
            format!("   iii trigger compose::add worker={name}"),
            styled_if(color, hint_style()),
        )));
    }
    for (key, label) in actions {
        lines.push(Line::from(vec![
            Span::styled(
                format!("   {key:<8}"),
                styled_if(color, Style::default().fg(Color::Cyan)),
            ),
            Span::raw(label),
        ]));
    }
    lines.push(Line::from(""));

    draw_dialog(
        f,
        area,
        format!(" {name} · press a key, or Esc "),
        lines,
        Vec::new(),
        58,
        color,
    );
}

fn draw_help_overlay(f: &mut Frame, area: Rect, color: bool) {
    let keys = [
        ("↑ ↓  k j", "select row"),
        ("g G", "first / last row"),
        ("s", "start: up a container, or add a repo worker on demand"),
        (
            "x",
            "stop: down a stack container, or drop an on-demand one",
        ),
        ("r", "compose::restart — this container only"),
        ("Ctrl+u", "compose::up the whole project"),
        ("w", "toggle injectable-UI watch (hot reload)"),
        ("d", "start_after + dependents"),
        ("f", "toggle live log follow"),
        ("PgUp PgDn", "scroll logs"),
        ("+ -", "resize the log pane"),
        ("/", "filter workers by name"),
        ("a", "browse for another directory of workers (remembered)"),
        (
            "Esc",
            "cancel the project start (only the cold boot can be)",
        ),
        ("Enter", "what this row can do"),
        ("?", "toggle this help"),
        ("q", "quit (leave running, or stop everything)"),
    ];
    let mut lines = vec![Line::from("")];
    for (key, description) in keys {
        lines.push(Line::from(vec![
            Span::styled(
                format!("   {key:<12}"),
                styled_if(color, Style::default().fg(Color::Cyan)),
            ),
            Span::raw(description),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("   ● ", styled_if(color, Style::default().fg(Color::Green))),
        Span::raw("ready       "),
        Span::styled("◐ ", styled_if(color, Style::default().fg(Color::Yellow))),
        Span::raw("starting / queued"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("   ✗ ", styled_if(color, Style::default().fg(Color::Red))),
        Span::raw("failed      "),
        Span::styled("○ ", styled_if(color, muted_cell_style())),
        Span::raw("stopped"),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "   mouse       ",
            styled_if(color, Style::default().fg(Color::Cyan)),
        ),
        Span::raw("click a row · wheel scrolls the pane under the pointer"),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "   stack = harness/worker-compose.yaml · repo = everything else here",
        styled_if(color, hint_style()),
    )));
    lines.push(Line::from(Span::styled(
        "   a repo worker you start is declared in worker-compose.local.yaml (gitignored)",
        styled_if(color, hint_style()),
    )));
    lines.push(Line::from(Span::styled(
        "   the compose (daemon) row logs the startup tree and every error code",
        styled_if(color, hint_style()),
    )));
    lines.push(Line::from(Span::styled(
        "   deeper history: iii compose logs <container> -F",
        styled_if(color, hint_style()),
    )));
    draw_dialog(
        f,
        area,
        " keys · any key to close ".to_string(),
        lines,
        Vec::new(),
        70,
        color,
    );
}

fn centered_rect_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn status_icon(state: &str, spinner_frame: usize) -> &'static str {
    match state {
        "ready" => "●",
        "starting" => SPINNER[spinner_frame % SPINNER.len()],
        "queued" => "◐",
        "failed" => "✗",
        _ => "○",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(name: &str, root: &str) -> crate::config::RepoWorker {
        crate::config::RepoWorker {
            name: name.to_string(),
            dir: PathBuf::from(root).join(name),
            root: PathBuf::from(root),
            declared: None,
            bin: Some(name.to_string()),
            ui_dir: None,
        }
    }

    /// One header per directory, in the order the directories were configured,
    /// and never a header for a directory the filter emptied.
    #[test]
    fn every_directory_gets_its_own_group() {
        let state = DashboardState {
            repo_workers: vec![
                worker("browser", "/repo"),
                worker("database", "/repo"),
                worker("harness-e2e", "/else/harness-e2e"),
            ],
            ..DashboardState::default()
        };

        let rows = build_rows(&state, "");
        assert!(matches!(rows[0], RowRef::Daemon));
        assert!(
            matches!(rows[1], RowRef::Group(0)),
            "the repo's own group comes first"
        );
        assert!(matches!(rows[2], RowRef::Repo(0)));
        assert!(matches!(rows[3], RowRef::Repo(1)));
        assert!(
            matches!(rows[4], RowRef::Group(2)),
            "a new root opens a new group"
        );
        assert!(matches!(rows[5], RowRef::Repo(2)));
        assert_eq!(rows.len(), 6);

        // The header is pushed on the first row that survives the filter, so
        // filtering a directory away takes its header with it.
        let filtered = build_rows(&state, "harness");
        assert!(matches!(filtered[0], RowRef::Daemon));
        assert!(matches!(filtered[1], RowRef::Group(2)));
        assert!(matches!(filtered[2], RowRef::Repo(2)));
        assert_eq!(filtered.len(), 3);
    }

    /// A list shorter than the box shows whole; a longer one scrolls with the
    /// cursor and never past its own end.
    #[test]
    fn the_browser_window_follows_the_cursor() {
        // Everything fits: always start at the top, wherever the cursor is.
        assert_eq!(browse_window(0, 4, 19), 0);
        assert_eq!(browse_window(3, 4, 19), 0);
        // Longer than the box: the cursor rides the bottom edge going down.
        assert_eq!(browse_window(0, 40, 10), 0);
        assert_eq!(browse_window(9, 40, 10), 0, "last row that still fits");
        assert_eq!(browse_window(10, 40, 10), 1);
        // And stops at the end rather than scrolling past it.
        assert_eq!(browse_window(39, 40, 10), 30);
    }

    /// Headers are targets now — `x` on one is how a directory is dropped.
    #[test]
    fn selection_lands_on_headers() {
        let rows = vec![
            RowRef::Daemon,
            RowRef::StackHeader,
            RowRef::Container(0),
            RowRef::Group(0),
            RowRef::Repo(0),
        ];
        let mut table = TableState::default();
        table.select(Some(0));
        move_selection(&mut table, &rows, true);
        assert_eq!(table.selected(), Some(1), "the stack header is selectable");
        move_selection(&mut table, &rows, true);
        move_selection(&mut table, &rows, true);
        assert_eq!(table.selected(), Some(3), "and so is a group header");

        table.select(Some(99));
        clamp_selection(&mut table, &rows);
        assert_eq!(table.selected(), Some(4));
    }

    /// What a directory offers is add-another and drop-this; the repo's own
    /// group cannot be dropped, so it does not say it can.
    #[test]
    fn a_group_header_offers_directory_actions() {
        let entry = |total: usize, offer: usize, added: bool| BrowseEntry {
            name: "x".into(),
            path: PathBuf::from("/x"),
            total,
            offer,
            added,
        };
        assert_eq!(entry(1, 1, false).label(), "1 worker");
        assert_eq!(entry(63, 63, false).label(), "63 workers");
        // The worktree case: everything it holds, the repo already provides.
        assert_eq!(entry(60, 0, false).label(), "0 new of 60");
        assert_eq!(entry(63, 4, false).label(), "4 new of 63");
        assert_eq!(entry(1, 1, true).label(), "added");
        assert_eq!(entry(0, 0, false).label(), "");
    }

    /// The matrix a guard refuses is the matrix the footer and the menu offer.
    /// Drift here means the screen advertises a key that does nothing.
    #[test]
    fn a_row_offers_only_what_its_guards_accept() {
        // is_container, local, watchable, in_stack, startable
        fn keys(actions: Vec<(&'static str, &'static str)>) -> Vec<&'static str> {
            actions.into_iter().map(|(key, _)| key).collect()
        }

        // The row selected at launch. Its own lifecycle keys are all no-ops.
        assert_eq!(keys(row_actions(false, false, false, true, false)), ["^u"]);
        // A stack container, with and without a watchable ui/.
        assert_eq!(
            keys(row_actions(true, false, true, true, false)),
            ["s", "x", "r", "w", "d"]
        );
        assert_eq!(
            keys(row_actions(true, false, false, true, false)),
            ["s", "x", "r", "d"]
        );
        // Started on demand: `r` would duplicate `s`, and `d` can only read
        // `(none)` twice off a project the stack file does not describe.
        assert_eq!(
            keys(row_actions(true, true, false, false, false)),
            ["s", "x"]
        );
        // Offered by the repo, never started.
        assert_eq!(keys(row_actions(false, false, false, false, true)), ["s"]);
        // Not a Rust binary: the menu says where to get it instead.
        assert!(keys(row_actions(false, false, false, false, false)).is_empty());
    }

    #[test]
    fn compose_phases_map_onto_the_table_states() {
        assert_eq!(normalize_phase("waiting for dependencies"), "queued");
        assert_eq!(normalize_phase("starting"), "starting");
        assert_eq!(normalize_phase("configuring"), "starting");
        assert_eq!(normalize_phase("registering"), "starting");
        assert_eq!(normalize_phase("ready"), "ready");
        assert_eq!(normalize_phase("already running"), "ready");
        assert_eq!(normalize_phase("failed"), "failed");
        assert_eq!(normalize_phase("rolled back"), "stopped");
    }
}
