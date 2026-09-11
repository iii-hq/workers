//! The dashboard. One table over `compose::status`, one log pane, and the
//! lifecycle keys — every one of them a `compose::*` call, none of them a
//! local process.

mod theme;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState};
use ratatui::{Frame, Terminal};

use crate::compose::{Compose, ContainerStatus, Progress};
use crate::config::LOG_TAIL_BYTES;
use crate::logs;

use theme::*;

const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];
const LOG_PAGE: usize = 10;
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

const HELP_FULL: &str =
    " s up · x down · r restart · w ui-watch · d deps · f follow · / filter · ? keys · q quit ";
const HELP_MID: &str = " s up · x down · r restart · / filter · ? keys · q quit ";
const HELP_MIN: &str = " / filter · ? keys · q quit ";

/// The row pinned above the containers. Its log is the compose daemon's own
/// output — the only place the startup tree, the adoption lines, the managed
/// engine's pid and every `error[CODE]` are ever printed.
const DAEMON_ROW: &str = "compose (daemon)";

enum UiMode {
    Dashboard,
    Filter,
    /// Typing a directory to offer workers from. Same shape as Filter — a
    /// footer prompt rather than a modal, so the list stays visible.
    AddDir(String),
    Help,
    Deps {
        name: String,
        deps: Vec<String>,
        dependents: Vec<String>,
    },
    ConfirmDown {
        name: String,
        dependents: Vec<String>,
    },
    Busy(String),
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModeKind {
    Dashboard,
    Filter,
    AddDir,
    Help,
    Deps,
    Confirm,
    Busy,
    Quit,
}

fn mode_kind(mode: &UiMode) -> ModeKind {
    match mode {
        UiMode::Dashboard => ModeKind::Dashboard,
        UiMode::Filter => ModeKind::Filter,
        UiMode::AddDir(_) => ModeKind::AddDir,
        UiMode::Help => ModeKind::Help,
        UiMode::Deps { .. } => ModeKind::Deps,
        UiMode::ConfirmDown { .. } => ModeKind::Confirm,
        UiMode::Busy(_) => ModeKind::Busy,
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
/// everything else the repo ships — running or not, so starting one is a
/// keypress rather than a file edit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowRef {
    Daemon,
    StackHeader,
    RepoHeader,
    Container(usize),
    Repo(usize),
}

impl RowRef {
    fn selectable(self) -> bool {
        !matches!(self, Self::StackHeader | Self::RepoHeader)
    }
}

struct Actions {
    compose: Arc<Compose>,
    in_flight: Arc<AtomicUsize>,
    errors: tokio::sync::mpsc::UnboundedSender<String>,
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
    execute!(io::stdout(), EnterAlternateScreen, Print("\x1b[22;0t"))?;

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
        let mut error_banner: Option<(String, Instant)> = None;
        let mut running = true;
        let mut stop_on_exit = false;
        let mut needs_redraw = true;
        let mut last_busy_tick = Instant::now();
        let mut last_redraw = Instant::now();
        let mut log_lines: Vec<String> = Vec::new();
        let mut log_title = String::new();
        let mut last_log_read = Instant::now() - Duration::from_secs(60);
        let mut last_log_key = String::new();

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
            if key != last_log_key || last_log_read.elapsed() >= poll_interval {
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
                    spinner_frame,
                    color_enabled,
                    error: error_banner.as_ref().map(|(s, _)| s.as_str()),
                };
                terminal.draw(|f| draw_ui(f, &mut table_state, &ctx))?;
                needs_redraw = false;
                last_redraw = Instant::now();
            }

            if last_busy_tick.elapsed() >= poll_interval {
                last_busy_tick = Instant::now();
                if matches!(mode, UiMode::Busy(_)) && in_flight.load(Ordering::SeqCst) == 0 {
                    mode = UiMode::Dashboard;
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
                            ModeKind::AddDir => handle_add_dir_key(key, &mut mode, &actions),
                            ModeKind::Help | ModeKind::Deps => mode = UiMode::Dashboard,
                            ModeKind::Confirm => handle_confirm_key(key, &mut mode, &actions),
                            ModeKind::Quit => {
                                handle_quit_key(key, &mut mode, &mut running, &mut stop_on_exit)
                            }
                            ModeKind::Busy => match key.code {
                                // `compose::cancel` is a control, not a mutation,
                                // so it answers during the cold build when up,
                                // down and restart do not.
                                KeyCode::Esc if progress.live() => spawn_cancel(&actions),
                                KeyCode::Esc if in_flight.load(Ordering::SeqCst) == 0 => {
                                    mode = UiMode::Dashboard
                                }
                                // An action is a detached call to a daemon that
                                // owns the lifecycle, so leaving while one runs
                                // is the architecture's default, not an escape
                                // hatch. Without this a `s` on a cold container
                                // holds the whole UI for the lifecycle timeout.
                                KeyCode::Char('q') => mode = UiMode::Quit,
                                _ => {}
                            },
                            ModeKind::Dashboard => handle_dashboard_key(
                                key,
                                &mut mode,
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
                    Event::Resize(_, _) => needs_redraw = true,
                    _ => {}
                }
            }

            if let Ok(message) = err_rx.try_recv() {
                error_banner = Some((message, Instant::now()));
                needs_redraw = true;
            }
            if error_banner
                .as_ref()
                .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(8))
            {
                error_banner = None;
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
    execute!(io::stdout(), Print("\x1b[23;0t"), LeaveAlternateScreen)?;
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

    let mut repo = Vec::new();
    for (index, worker) in state.repo_workers.iter().enumerate() {
        if !matches(&worker.name) {
            continue;
        }
        match state
            .containers
            .iter()
            .position(|entry| entry.local && entry.status.container == worker.name)
        {
            Some(container) => repo.push(RowRef::Container(container)),
            None => repo.push(RowRef::Repo(index)),
        }
    }
    if !repo.is_empty() {
        rows.push(RowRef::RepoHeader);
        rows.extend(repo);
    }
    rows
}

/// The project that owns a row: the tracked stack, or the on-demand file.
fn project_of(compose: &Compose, state: &DashboardState, row: RowRef) -> PathBuf {
    match row {
        RowRef::Container(index) => state.containers[index].file.clone(),
        _ => compose.config.compose_path.clone(),
    }
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
    let mut current = table_state.selected().unwrap_or(0).min(rows.len() - 1);
    // A group header is a label, not a target; land on the next real row.
    while current < rows.len() && !rows[current].selectable() {
        current += 1;
    }
    if current >= rows.len() {
        current = (0..rows.len())
            .rev()
            .find(|i| rows[*i].selectable())
            .unwrap_or(0);
    }
    table_state.select(Some(current));
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

fn start_on_demand(actions: &Actions, mode: &mut UiMode, name: String) {
    match actions
        .compose
        .repo_worker(&name)
        .and_then(|worker| worker.bin.clone())
    {
        Some(bin) => {
            *mode = UiMode::Busy(format!("starting {name}…"));
            spawn_add_local(actions, name, bin);
        }
        None => {
            *mode = UiMode::Busy(format!(
                "{name} installs from the registry, not from this tree"
            ))
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
        KeyCode::Char('/') => *mode = UiMode::Filter,
        KeyCode::Char('a') => *mode = UiMode::AddDir(String::new()),
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
            *log_scroll += LOG_PAGE;
        }
        KeyCode::PageDown => {
            *log_scroll = log_scroll.saturating_sub(LOG_PAGE);
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
            *mode = UiMode::Busy("starting the project…".to_string());
            spawn_up(actions, compose.config.compose_path.clone(), None);
        }
        KeyCode::Char('s') => match (name, running) {
            // An on-demand worker is re-declared on every start, so the
            // declaration follows the repo and the daemon re-reads it.
            (Some(name), Some(_)) if is_local => start_on_demand(actions, mode, name),
            (Some(name), Some(file)) => {
                *mode = UiMode::Busy(format!("up {name}…"));
                spawn_up(actions, file, Some(name));
            }
            // A repo worker nothing declares yet: declare it in its own
            // project and start it, which is the whole point of listing them.
            (Some(name), None) => start_on_demand(actions, mode, name),
            _ => {}
        },
        KeyCode::Char('x') => {
            if let (Some(name), Some(file)) = (name, running) {
                // An on-demand worker has no dependents to take down with it:
                // nothing in the stack can declare a dependency on it.
                if is_local {
                    *mode = UiMode::Busy(format!("down {name}…"));
                    spawn_down(actions, file, name);
                } else {
                    let dependents = compose.config.dependents(&name);
                    *mode = UiMode::ConfirmDown { name, dependents };
                }
            }
        }
        KeyCode::Char('r') => {
            if let (Some(name), Some(file)) = (name, running) {
                *mode = UiMode::Busy(format!("restarting {name}…"));
                spawn_restart(actions, file, name);
            }
        }
        KeyCode::Char('w') => {
            if let (Some(name), Some(file)) = (name, running) {
                *mode = UiMode::Busy(format!("ui watch {name}…"));
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

fn move_selection(table_state: &mut TableState, rows: &[RowRef], down: bool) {
    let current = table_state.selected().unwrap_or(0);
    let next = if down {
        (current + 1..rows.len()).find(|i| rows[*i].selectable())
    } else {
        (0..current).rev().find(|i| rows[*i].selectable())
    };
    if let Some(next) = next {
        table_state.select(Some(next));
    }
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
fn handle_add_dir_key(key: KeyEvent, mode: &mut UiMode, actions: &Actions) {
    let UiMode::AddDir(input) = mode else {
        return;
    };
    match key.code {
        KeyCode::Enter => {
            let input = input.clone();
            let compose = actions.compose.clone();
            *mode = UiMode::Busy(format!("scanning {input}…"));
            spawn_action(actions, async move {
                compose.add_worker_dir(&input).map(|_| ())
            });
        }
        KeyCode::Esc => *mode = UiMode::Dashboard,
        KeyCode::Backspace => {
            input.pop();
        }
        KeyCode::Tab => complete_path(input),
        KeyCode::Char(ch) => input.push(ch),
        _ => {}
    }
}

/// Extend the typed path by the longest prefix every matching directory shares,
/// and add the separator when exactly one matches.
fn complete_path(input: &mut String) {
    let expanded = crate::config::expand_home(input);
    let (dir, prefix) = if input.ends_with('/') {
        (expanded.as_path(), String::new())
    } else {
        (
            expanded.parent().unwrap_or(Path::new("/")),
            expanded
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default(),
        )
    };

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let matches: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            name.starts_with(&prefix).then_some(name)
        })
        .collect();
    let Some(first) = matches.first() else {
        return;
    };

    let mut shared = first.clone();
    for candidate in &matches {
        while !candidate.starts_with(&shared) {
            shared.pop();
        }
    }
    if shared.len() <= prefix.len() {
        return;
    }
    input.truncate(input.len() - prefix.len());
    input.push_str(&shared);
    if matches.len() == 1 {
        input.push('/');
    }
}

fn handle_confirm_key(key: KeyEvent, mode: &mut UiMode, actions: &Actions) {
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
            *mode = UiMode::Busy(format!("down {name}…"));
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

fn draw_ui(f: &mut Frame, table_state: &mut TableState, ctx: &UiCtx) {
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
        draw_table(f, rows[0], table_state, ctx);
        draw_log_pane(f, rows[1], ctx);
    }

    draw_footer(f, chunks[2], ctx);

    match ctx.mode {
        UiMode::ConfirmDown { name, dependents } => {
            draw_confirm_overlay(f, body, name, dependents, ctx)
        }
        UiMode::Busy(message) => draw_busy_overlay(f, body, message, ctx),
        UiMode::Deps {
            name,
            deps,
            dependents,
        } => draw_deps_overlay(f, body, name, deps, dependents, ctx),
        UiMode::Help => draw_help_overlay(f, area, ctx.color_enabled),
        UiMode::Quit => draw_quit_overlay(f, body, ctx),
        _ => {}
    }
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

    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        &ctx.compose.config.engine_url,
        styled_if(color, engine_url_style()),
    ));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("ns:{}", ctx.compose.config.namespace),
        styled_if(color, muted_cell_style()),
    ));

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
            RowRef::RepoHeader => group_row(ctx, "repo", RowRef::RepoHeader, color),
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
                        entry.status.last_error.clone().unwrap_or_default(),
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
        .take_while(|row| row.selectable())
        .count();
    Row::new(vec![Cell::from(Span::styled(
        format!("── {label} ({count}) ──"),
        styled_if(color, hint_style()),
    ))])
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
    while lines.len() < inner_height {
        lines.push(Line::from(" ".repeat(inner_width.max(1))));
    }
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
    if let Some(error) = ctx.error {
        f.render_widget(
            Paragraph::new(format!(" ⚠ {error} ")).style(
                styled_if(color, Style::default().fg(Color::Red)).add_modifier(Modifier::BOLD),
            ),
            area,
        );
        return;
    }
    let (text, style) = match ctx.mode {
        UiMode::Filter => (
            format!(" filter: {}_   (Enter apply · Esc clear) ", ctx.filter),
            styled_if(color, Style::default().fg(Color::Yellow)),
        ),
        UiMode::AddDir(input) => (
            format!(" worker dir: {input}_   (Tab complete · Enter add · Esc cancel) "),
            styled_if(color, Style::default().fg(Color::Cyan)),
        ),
        _ => {
            // Gate on the string's own width, not a hand-copied number — the
            // full help outgrew its old gate once and clipped its own tail.
            let help = if area.width as usize >= HELP_FULL.chars().count() {
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
        ("a", "offer the workers in another directory (remembered)"),
        ("Esc", "cancel the running operation (while busy)"),
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

fn draw_busy_overlay(f: &mut Frame, area: Rect, message: &str, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let spin = SPINNER[ctx.spinner_frame % SPINNER.len()];
    let mut text = format!("{message}  ");
    if ctx.progress.live() {
        text.push_str("(Esc cancels)  ");
    }
    let line = Line::from(vec![
        Span::styled(
            format!("  {spin} "),
            styled_if(color, Style::default().fg(Color::Yellow)),
        ),
        Span::raw(text.clone()),
    ]);
    let width = (text.chars().count() as u16).saturating_add(8).max(24);
    let popup = centered_rect_fixed(width, 3, area);
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

    /// stack + repo means the table now carries rows that are labels, not
    /// targets. Every index path has to step over them.
    #[test]
    fn selection_steps_over_the_group_headers() {
        let rows = vec![
            RowRef::Daemon,
            RowRef::StackHeader,
            RowRef::Container(0),
            RowRef::RepoHeader,
            RowRef::Repo(0),
            RowRef::Repo(1),
        ];
        let mut table = TableState::default();

        table.select(Some(0));
        move_selection(&mut table, &rows, true);
        assert_eq!(table.selected(), Some(2), "skips the stack header");
        move_selection(&mut table, &rows, true);
        assert_eq!(table.selected(), Some(4), "skips the repo header");
        move_selection(&mut table, &rows, false);
        assert_eq!(table.selected(), Some(2), "and skips it going back");

        // `G` lands on the last row, `g` on the first; neither may be a header.
        table.select(Some(rows.len() - 1));
        clamp_selection(&mut table, &rows);
        assert_eq!(table.selected(), Some(5));
        table.select(Some(1));
        clamp_selection(&mut table, &rows);
        assert_eq!(table.selected(), Some(2));
    }

    /// A filter that hides a whole group drops its header with it, and the
    /// daemon row — the only place the daemon's own errors show — always stays.
    #[test]
    fn a_group_with_no_rows_keeps_no_header() {
        let rows = vec![RowRef::Daemon];
        let mut table = TableState::default();
        table.select(Some(3));
        clamp_selection(&mut table, &rows);
        assert_eq!(table.selected(), Some(0));
        assert!(RowRef::Daemon.selectable());
        assert!(!RowRef::StackHeader.selectable());
        assert!(!RowRef::RepoHeader.selectable());
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
