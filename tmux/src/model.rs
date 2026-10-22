//! Wire shapes shared by `tmux::sessions::list`, `tmux::ports::list` and the
//! `tmux::changed` live feed. Timestamps are milliseconds since the epoch.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Whether a usable tmux exists and which server this worker talks to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TmuxInfo {
    /// A usable tmux binary was found.
    pub available: bool,
    /// Resolved tmux binary.
    pub path: Option<String>,
    /// tmux version, e.g. `3.6`.
    pub version: Option<String>,
    /// Server socket, known while the server runs.
    pub socket_path: Option<String>,
    /// A tmux server is running on that socket.
    pub server_running: bool,
    /// Why tmux is unavailable, or the last listing error.
    pub error: Option<String>,
}

/// Every session with its windows and panes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionsSnapshot {
    /// tmux availability and server.
    pub tmux: TmuxInfo,
    /// Sessions sorted by name.
    pub sessions: Vec<Session>,
    /// When the snapshot was taken.
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Session {
    /// Session id, e.g. `$5`.
    pub id: String,
    /// Session name.
    pub name: String,
    /// When the session was created.
    pub created_at_ms: u64,
    /// Last activity in any of its windows.
    pub activity_at_ms: u64,
    /// tmux clients attached, console terminals included.
    pub attached: u32,
    /// How many attached clients are console terminals.
    pub console_attached: u32,
    /// Windows sorted by index.
    pub windows: Vec<Window>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Window {
    /// Window id, e.g. `@3` (unique on the server).
    pub id: String,
    /// Window index inside the session.
    pub index: u32,
    /// Window name.
    pub name: String,
    /// This is the session's current window.
    pub active: bool,
    /// Last output in the window.
    pub activity_at_ms: u64,
    /// A bell rang in the window.
    pub bell: bool,
    /// A pane is zoomed.
    pub zoomed: bool,
    /// Panes sorted by index.
    pub panes: Vec<Pane>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pane {
    /// Pane id, e.g. `%5`.
    pub id: String,
    /// Pane index inside the window.
    pub index: u32,
    /// The window's active pane.
    pub active: bool,
    /// Pid of the process tmux started in the pane.
    pub pid: u32,
    /// Foreground process name.
    pub command: String,
    /// Current directory.
    pub cwd: String,
    /// Pane title set by the program.
    pub title: String,
    /// Width in columns.
    pub width: u32,
    /// Height in rows.
    pub height: u32,
    /// The pane's process exited and the pane is kept.
    pub dead: bool,
    /// The agent CLI running in this pane, if any.
    pub agent: Option<AgentStatus>,
    /// The `iii compose` daemon running in this pane, if any.
    pub compose: Option<ComposeRef>,
    /// Listening sockets owned by processes started from this pane.
    pub ports: Vec<Listener>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// The agent is running a turn.
    Working,
    /// The agent shows a prompt that needs you.
    Waiting,
    /// The agent waits for a new message.
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentStatus {
    /// Matcher id from the configuration, e.g. `claude`.
    pub id: String,
    /// Display name, e.g. `Claude Code`.
    pub label: String,
    /// Pid of the agent process.
    pub pid: u32,
    /// What the agent is doing.
    pub state: AgentState,
    /// When the agent entered `state`.
    pub since_ms: u64,
    /// The agent's session title: the pane title without the agent's status
    /// glyph (Claude Code's ✳ or spinner). Null when the pane only shows
    /// tmux's default title (the host name) or nothing.
    #[serde(default)]
    pub title: Option<String>,
    /// Sub-agents working right now, oldest first. Their work keeps `state`
    /// at `working` after the agent's own turn ended. Claude Code and Codex
    /// report them; other agents always list none.
    #[serde(default)]
    pub subagents: Vec<SubagentStatus>,
}

/// A sub-agent at work under an agent CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubagentStatus {
    /// Stable within the agent's session: Claude Code's agent id, Codex's thread id.
    pub id: String,
    /// What it was asked to do: Claude Code's task description, Codex's task
    /// path under the main agent (`review/tests`).
    pub description: String,
    /// Claude Code's agent type (`general-purpose`, `Explore`, `fork`) or
    /// Codex's agent role; null when the agent names none.
    pub kind: Option<String>,
    /// 1 for a sub-agent of the main agent, 2 for one of its sub-agents, ….
    pub depth: u32,
    /// When it started its current task.
    pub since_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ComposeRef {
    /// Compose daemon namespace.
    pub namespace: String,
    /// Compose daemon pid.
    pub pid: u32,
}

/// Where a process runs in tmux.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PaneRef {
    /// Session name.
    pub session: String,
    /// Session id.
    pub session_id: String,
    /// Window index.
    pub window_index: u32,
    /// Window id.
    pub window_id: String,
    /// Pane id.
    pub pane_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ListenerRole {
    /// An iii engine (its WebSocket and built-in listeners).
    Engine,
    /// An iii worker.
    Worker,
    /// An `iii compose` daemon.
    Compose,
    /// Anything else.
    Other,
}

/// One TCP socket in LISTEN state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Listener {
    /// Port number.
    pub port: u16,
    /// Bound address, e.g. `0.0.0.0` or `::1`.
    pub address: String,
    /// IPv4 or IPv6.
    pub family: AddressFamily,
    /// Owning process.
    pub pid: u32,
    /// Owning process name.
    pub process: String,
    /// iii worker name, when known.
    pub worker: Option<String>,
    /// Compose namespace the process belongs to.
    pub compose: Option<String>,
    /// What kind of process owns the socket.
    pub role: ListenerRole,
    /// tmux pane the process descends from.
    pub location: Option<PaneRef>,
}

/// Listening ports and the `iii compose` stacks that own them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PortsSnapshot {
    /// This platform has a port scanner.
    pub supported: bool,
    /// Why the scan failed, if it did.
    pub error: Option<String>,
    /// Running `iii compose` daemons.
    pub composes: Vec<ComposeStack>,
    /// Every TCP listening socket of this user's processes, sorted by port.
    pub listeners: Vec<Listener>,
    /// When the snapshot was taken.
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ComposeStack {
    /// Daemon namespace (`iii:c:<namespace>`).
    pub namespace: String,
    /// Daemon pid.
    pub pid: u32,
    /// Daemon working directory.
    pub project_dir: Option<String>,
    /// Compose file the daemon serves, when known.
    pub compose_file: Option<String>,
    /// The engine the daemon started, if it manages one.
    pub engine: Option<ComposeEngine>,
    /// Workers the daemon started.
    pub workers: Vec<ComposeWorker>,
    /// tmux pane the daemon runs in.
    pub location: Option<PaneRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ComposeEngine {
    /// Engine pid.
    pub pid: u32,
    /// Ports the engine listens on.
    pub ports: Vec<u16>,
    /// The engine WebSocket port workers connect to.
    pub ws_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ComposeWorker {
    /// Worker name.
    pub name: String,
    /// Top-most pid of the worker's process tree.
    pub pid: u32,
    /// Ports the worker's processes listen on.
    pub ports: Vec<u16>,
}

/// Payload of the `tmux::changed` live feed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Overview {
    /// Sessions, windows, panes and their status.
    pub sessions: SessionsSnapshot,
    /// Listening ports and compose stacks.
    pub ports: PortsSnapshot,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}
