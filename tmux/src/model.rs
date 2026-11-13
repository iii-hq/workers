//! Wire shapes shared by `tmux::sessions::list`, `tmux::ports::list` and the
//! `tmux::on-changed` live feed. Timestamps are milliseconds since the epoch.

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
    /// tmux is missing and `tmux::install` can try: a supported OS, the
    /// configured binary is plain `tmux`, not a path or another name, and this
    /// processor has an official build or this computer a package manager.
    #[serde(default)]
    pub installable: bool,
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
    /// The Claude Code account the pane runs on and how it follows the
    /// global one. Present when the pane runs Claude Code or carries an
    /// account mark; absent otherwise and from workers before 0.3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<PaneClaude>,
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

/// What one sub-agent has done so far, from the transcript its agent keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubagentTranscript {
    /// Pane id of the agent running the sub-agent.
    pub pane_id: String,
    /// Matcher id of that agent: `claude` or `codex`.
    pub agent: String,
    /// The sub-agent's id, as in `agent.subagents`.
    pub subagent_id: String,
    /// The sub-agent is still in its agent's `subagents` list.
    pub working: bool,
    /// The task it was given, cut at 16 000 characters. Null when the
    /// transcript holds none; Codex keeps a sub-agent's task encrypted, so
    /// there it shows only the task's name and sender.
    pub prompt: Option<String>,
    /// The latest items, oldest first, without the task.
    pub items: Vec<TranscriptItem>,
    /// Older items were left out: past `limit`, or before the last 4 MiB of
    /// the file.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptItemKind {
    /// A message to the sub-agent after its task, from the agent that runs it.
    User,
    /// What the sub-agent wrote.
    Text,
    /// Its reasoning, when the agent records it in clear.
    Thinking,
    /// A tool it called, with the call's main argument.
    ToolCall,
    /// What a tool returned.
    ToolResult,
}

/// One step of a sub-agent's transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TranscriptItem {
    pub kind: TranscriptItemKind,
    /// When the agent recorded it.
    pub at_ms: Option<u64>,
    /// The text, cut at 4 000 characters (then ending in `…`). A tool call's
    /// is its main argument: the command, path, pattern or query, else the
    /// whole input as JSON.
    pub text: String,
    /// The tool's name, on `tool_call` and `tool_result` items.
    pub tool: Option<String>,
    /// The tool reported a failure (`tool_result` only).
    #[serde(default)]
    pub error: bool,
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

/// What `tmux::on-changed` returns: the payload of the live feed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Overview {
    /// Sessions, windows, panes and their status.
    pub sessions: SessionsSnapshot,
    /// Listening ports and compose stacks.
    pub ports: PortsSnapshot,
    /// Claude Code accounts and the weekly usage of Claude Code and Codex.
    /// Absent from workers before 0.3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageSnapshot>,
}

// ── Claude Code accounts and agent usage ────────────────────────────────

/// How a pane's Claude Code account is chosen (agent-accounts G12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Follow {
    /// Runs on the global slot (`~/.claude`) and is not pinned, started by
    /// the worker or by hand: it switches with the global account on its
    /// next message.
    Global,
    /// Pinned to an account: on its folder, or on the slot while that
    /// account is global (it restarts on its folder when the global one
    /// changes).
    Pinned,
    /// Started by hand on an account's folder (`CLAUDE_CONFIG_DIR` set): no mark.
    Manual,
}

/// The Claude Code account of one pane (agent-accounts D4, D5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PaneClaude {
    /// Id of the account the running Claude Code uses: the global account
    /// on the slot (`~/.claude`), else the account whose folder its
    /// `CLAUDE_CONFIG_DIR` names. Null when that folder is no registered
    /// account (see `config_dir`) or Claude Code is not running.
    #[serde(default)]
    pub account: Option<String>,
    /// The config folder the running Claude Code uses, `~` for the home
    /// folder. Null when Claude Code is not running.
    #[serde(default)]
    pub config_dir: Option<String>,
    /// `global`, `pinned` or `manual`.
    pub follow: Follow,
    /// The account the pane moves to when its turn ends (`global`: the slot).
    #[serde(default)]
    pub pending: Option<String>,
    /// Claude Code runs without permission prompts
    /// (`--dangerously-skip-permissions` or `--permission-mode bypassPermissions`).
    pub skip_permissions: bool,
    /// Claude Code's session id, which a move resumes.
    #[serde(default)]
    pub session_id: Option<String>,
    /// The worker can move this pane to another account.
    pub movable: bool,
    /// Why it can't, in one sentence, when `movable` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_movable_reason: Option<String>,
    /// Why the last move of this pane (one that waited for its turn, or a
    /// switch of the global account) failed, in one sentence; cleared by
    /// the next account change of the pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_error: Option<String>,
}

/// One usage window of an account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Limit {
    /// How much of the window is used, 0 to 100.
    pub used_percent: f64,
    /// When the window starts again.
    #[serde(default)]
    pub resets_at_ms: Option<u64>,
    /// The window reset after the reading, so `used_percent` is 0 until the
    /// next reading.
    #[serde(default)]
    pub reset_since_read: bool,
}

/// A weekly window of one model family (Claude Code's `weekly_scoped` limits).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelLimit {
    /// The model's display name, e.g. `Opus`.
    pub name: String,
    /// The window.
    pub limit: Limit,
}

/// Usage beyond the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExtraUsage {
    /// Extra usage is turned on for the account.
    pub enabled: bool,
    /// How much of its monthly limit is used, 0 to 100, when there is one.
    #[serde(default)]
    pub used_percent: Option<f64>,
}

/// One reading of a Claude Code account's usage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClaudeUsage {
    /// The current 5-hour window.
    #[serde(default)]
    pub five_hour: Option<Limit>,
    /// The week, all models.
    #[serde(default)]
    pub week: Option<Limit>,
    /// The week, Opus only.
    #[serde(default)]
    pub week_opus: Option<Limit>,
    /// The week, Sonnet only.
    #[serde(default)]
    pub week_sonnet: Option<Limit>,
    /// Every per-model weekly window the account has, Opus and Sonnet included.
    #[serde(default)]
    pub week_models: Vec<ModelLimit>,
    /// Usage beyond the plan.
    #[serde(default)]
    pub extra: Option<ExtraUsage>,
    /// When this reading was taken.
    pub as_of_ms: u64,
    /// Why the latest attempt to read failed: `rate_limited`, `unauthorized`,
    /// `network`, `token_expired`, `not_signed_in`, `unreadable`. The windows
    /// are then the last good reading, if any.
    #[serde(default)]
    pub error: Option<String>,
}

/// A Claude Code account (agent-accounts G1, G2): one sign-in, kept in the
/// global slot while it is global and in its own folder otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClaudeAccount {
    /// `default` for the sign-in `~/.claude` had when accounts began, else
    /// a slug of its first name.
    pub id: String,
    /// The name people gave it.
    pub name: String,
    /// `blue`, `green`, `violet`, `orange` or `pink`.
    pub color: String,
    /// Its folder, absolute: `<accounts root>/<id>`. Without `accounts.json`, the one account (`default`) has none, and this is `~/.claude`.
    pub dir: String,
    /// Its folder with `~` for the home folder.
    pub dir_display: String,
    /// Its id is `default`.
    pub is_default: bool,
    /// The signed-in email.
    #[serde(default)]
    pub email: Option<String>,
    /// The organization it belongs to.
    #[serde(default)]
    pub organization: Option<String>,
    /// `pro`, `max`, `team`, `enterprise` or what Claude Code recorded.
    #[serde(default)]
    pub plan: Option<String>,
    /// The rate-limit tier, e.g. `default_claude_max_20x`.
    #[serde(default)]
    pub tier: Option<String>,
    /// It holds a Claude sign-in.
    pub signed_in: bool,
    /// Its sign-in expired; running Claude Code there renews it.
    pub token_expired: bool,
    /// The groups its folder shares with `~/.claude`: `settings`,
    /// `extensions`, `plugins`, `conversations`.
    pub shared: Vec<String>,
    /// Claude Code panes using it now (for the global account, every pane
    /// on the slot, its followers included).
    pub panes: u32,
    /// The latest usage reading.
    #[serde(default)]
    pub usage: Option<ClaudeUsage>,
}

/// What the worker does when the global account runs low.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RulesMode {
    /// Nothing.
    Off,
    /// Suggest another account.
    #[default]
    Suggest,
    /// Make another account global by itself.
    Switch,
}

/// When the global account counts as low.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Threshold {
    /// 80 % of the week.
    #[serde(rename = "week_80")]
    Week80,
    /// 90 % of the week.
    #[default]
    #[serde(rename = "week_90")]
    Week90,
    /// The week's limit.
    Limit,
}

/// The rules for the global account (agent-accounts D14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SwitchRules {
    /// `off`, `suggest` or `switch`.
    pub mode: RulesMode,
    /// `week_80`, `week_90` or `limit`.
    pub at: Threshold,
    /// Also act when the 5-hour window reaches 100 %.
    pub five_hour: bool,
    /// After switching away by itself, switch back once that account's window resets.
    pub back_after_reset: bool,
    /// Accounts to try first, in order; then the one with most room this week.
    pub order: Vec<String>,
}

impl Default for SwitchRules {
    fn default() -> Self {
        Self {
            mode: RulesMode::Suggest,
            at: Threshold::Week90,
            five_hour: false,
            back_after_reset: false,
            order: Vec::new(),
        }
    }
}

/// Another account suggested for global.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Suggestion {
    /// Pass it to `tmux::accounts::dismiss`.
    pub id: String,
    /// The global account.
    pub from: String,
    /// The account suggested.
    pub to: String,
    /// `week_80`, `week_90`, `limit` or `five_hour`.
    pub reason: String,
    /// Claude Code panes on the slot that are not pinned: they switch with
    /// the global account.
    pub following: u32,
    /// When the reason was first seen.
    pub at_ms: u64,
}

/// The latest change of the global account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AccountSwitch {
    /// The global account before.
    pub from: String,
    /// The global account after.
    pub to: String,
    /// `week_80`, `week_90`, `limit`, `five_hour` or `back` for an
    /// automatic switch; `login` (someone signed in to a known account with
    /// `/login`), `login_new` (to a new one, added then), `upgrade` (the
    /// worker took the slot's sign-in for global when it was updated); null
    /// when someone switched.
    #[serde(default)]
    pub reason: Option<String>,
    /// The worker switched by itself (rules mode `switch`).
    pub auto: bool,
    /// Claude Code panes on the slot that switched with it, on their next
    /// message.
    #[serde(default)]
    pub following: u32,
    /// Pinned panes restarted: the new global account's (now on the slot)
    /// and the old one's (now on its folder).
    #[serde(default, alias = "moved")]
    pub restarted: Vec<String>,
    /// Panes pinned to the old global account that restart on its folder
    /// when their turn ends.
    pub after_turn: Vec<String>,
    /// When it happened.
    pub at_ms: u64,
    /// With `back_after_reset`: when `from`'s window resets and the worker
    /// switches back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back_at_ms: Option<u64>,
}

/// Claude Code's accounts as the worker sees them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClaudeSnapshot {
    /// A `claude` binary is on PATH or `~/.claude` exists.
    pub available: bool,
    /// The accounts registry exists (an account was added or a setting saved).
    pub managed: bool,
    /// Id of the global account: the sign-in of the slot (`~/.claude`), what
    /// plain `claude` uses anywhere.
    pub global: String,
    /// The worker runs as root, where Claude Code refuses `--dangerously-skip-permissions`.
    pub runs_as_root: bool,
    /// The default account first, then the others in the order they were added.
    pub accounts: Vec<ClaudeAccount>,
    /// The rules for the global account.
    pub rules: SwitchRules,
    /// Another account suggested for global, while the global one is low.
    #[serde(default)]
    pub suggestion: Option<Suggestion>,
    /// The latest change of the global account.
    #[serde(default)]
    pub last_switch: Option<AccountSwitch>,
    /// A switch of the global account that waits for windows on the new
    /// account's folder to finish their turn.
    #[serde(default)]
    pub pending_global: Option<PendingGlobal>,
    /// How long a running Claude Code may take to notice a switch, in
    /// seconds: 0 on Linux (its next message), 30 on macOS (the Keychain).
    #[serde(default)]
    pub switch_delay_s: u32,
    /// Sign-ins under way (`tmux::accounts::login_start`), and those that
    /// ended in the last minute, oldest first.
    #[serde(default)]
    pub logins: Vec<ClaudeLogin>,
}

/// Who may open the sign-in page on the computer the worker runs on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoginBrowser {
    /// Nobody: Claude Code opens no browser; the link is shown elsewhere and
    /// the code from the page is pasted back.
    #[default]
    None,
    /// Claude Code opens the browser there, whose return to its local
    /// listener finishes the sign-in with no code (the app runs on that
    /// computer).
    Host,
}

/// Where a sign-in is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoginState {
    /// Claude Code shows the link and waits for the code.
    WaitingCode,
    /// Claude Code is checking a code with Anthropic.
    Checking,
    /// Signed in.
    Done,
    /// It ended without a sign-in (`error` says why).
    Failed,
    /// The link's 10 minutes ran out; Claude Code was stopped.
    Expired,
}

/// Why a sign-in failed, as `tmux::accounts::login_code` answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LoginError {
    /// `rejected` (Anthropic didn't accept the code; Claude Code ended),
    /// `expired`, `gone` or `failed`.
    pub code: String,
    /// One sentence.
    pub message: String,
}

/// A sign-in of a Claude Code account from the app (claude-login-ui).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClaudeLogin {
    /// Pass it to `tmux::accounts::login_code` and `login_cancel`.
    pub login_id: String,
    /// The account signing in.
    pub account: String,
    pub state: LoginState,
    /// The sign-in page to open on any device: it shows the code to paste.
    pub url: String,
    /// With `browser: host`, the link Claude Code gave the browser on the
    /// worker's computer: its return to Claude Code's local listener
    /// finishes the sign-in with no code (it works on that computer only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_url: Option<String>,
    /// When the link stops working and Claude Code is stopped.
    pub expires_at_ms: u64,
    pub browser: LoginBrowser,
    /// The email the sign-in page is filled with (the account's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// With `failed` and `expired`: why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<LoginError>,
}

/// A switch of the global account waiting for windows to finish their turn
/// (agent-accounts G4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PendingGlobal {
    /// The account becoming global.
    pub account: String,
    /// `person` or `rule`.
    pub by: String,
    /// The Claude Code panes on its folder still working or waiting.
    pub waiting_for: Vec<String>,
    /// When it was asked for.
    pub at_ms: u64,
}

/// Codex's credits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CodexCredits {
    /// The balance as Codex reports it, e.g. `62500.0000000000`.
    pub balance: String,
    /// Credits are unlimited.
    pub unlimited: bool,
}

/// Codex's usage, from the newest session it recorded (agent-accounts D13).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CodexUsage {
    /// The ChatGPT plan, e.g. `pro`.
    #[serde(default)]
    pub plan: Option<String>,
    /// The 5-hour window, when the plan has one.
    #[serde(default)]
    pub five_hour: Option<Limit>,
    /// The week.
    #[serde(default)]
    pub week: Option<Limit>,
    /// Credits, when the plan has them.
    #[serde(default)]
    pub credits: Option<CodexCredits>,
    /// When Codex recorded it.
    pub as_of_ms: u64,
}

/// Claude Code accounts and the weekly usage of Claude Code and Codex.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UsageSnapshot {
    /// Claude Code's accounts and their usage.
    pub claude: ClaudeSnapshot,
    /// Codex's usage, when Codex recorded any.
    #[serde(default)]
    pub codex: Option<CodexUsage>,
    /// When the snapshot was taken.
    pub generated_at_ms: u64,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}
