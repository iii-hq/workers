//! Running tmux commands and reading their output.
//!
//! Every listing uses `-F` formats whose fields are separated by U+001F (unit
//! separator), which no session, window or path name contains in practice.
//! Some tmux builds (Ubuntu's 3.4, for one) print it as the four characters
//! `\037`; [`unescape_separators`] turns those back. The free-text field (pane
//! title) is always last so a stray separator in it cannot shift the others.

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::WorkerConfig;

pub const SEP: char = '\x1f';

/// What a new pane that keeps its dead program runs until
/// [`Tmux::keep_when_done`] starts the real one: waits, prints nothing.
pub const HOLD: &str = "cat";

/// [`SEP`] as tmux prints it when it passes command output through `vis(3)`
/// (octal, backslashes left alone).
const ESCAPED_SEP: &str = "\\037";

/// One line of `-F` output with its separators raw. A line that already holds
/// a raw separator came from a tmux that prints it untouched and is kept as is.
pub fn unescape_separators(line: &str) -> Cow<'_, str> {
    if line.contains(SEP) || !line.contains(ESCAPED_SEP) {
        Cow::Borrowed(line)
    } else {
        Cow::Owned(line.replace(ESCAPED_SEP, &SEP.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Socket {
    Default,
    Name(String),
    Path(String),
}

impl Socket {
    pub fn from_config(config: &WorkerConfig) -> Self {
        match (&config.socket_path, &config.socket_name) {
            (Some(path), _) => Self::Path(path.clone()),
            (None, Some(name)) => Self::Name(name.clone()),
            (None, None) => Self::Default,
        }
    }

    pub fn args(&self) -> Vec<String> {
        match self {
            Self::Default => Vec::new(),
            Self::Name(name) => vec!["-L".to_string(), name.clone()],
            Self::Path(path) => vec!["-S".to_string(), path.clone()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxError {
    /// No server listens on the socket (nothing to list yet).
    NoServer,
    /// tmux ran and refused, with its own message.
    Failed(String),
    /// tmux could not be run at all.
    Io(String),
    Timeout,
}

impl TmuxError {
    /// The refusal in one sentence for people. A target tmux can't find
    /// (`can't find pane: %9`; `can't find pane %9` before tmux 3.0) names
    /// what is missing; a socket tmux can't reach keeps its path out; anything
    /// else is tmux's own words.
    pub fn sentence(&self) -> String {
        match self {
            Self::NoServer => "No tmux server is running.".to_string(),
            Self::Timeout => "tmux did not answer in time.".to_string(),
            Self::Io(message) => format!("tmux could not be run ({message})."),
            Self::Failed(message) => failed_sentence(message),
        }
    }
}

fn failed_sentence(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default().trim();
    let missing = |what: &str| {
        line.strip_prefix(&format!("can't find {what}"))
            .map(|rest| rest.trim_start_matches(':').trim())
            .filter(|target| !target.is_empty())
    };
    if let Some(pane) = missing("pane") {
        return no_pane(pane);
    }
    if let Some(window) = missing("window") {
        return format!("There is no window {window}.");
    }
    if let Some(session) = missing("session") {
        return no_session(session);
    }
    // `can't find client: /dev/pts/3`: the console's tmux client ended.
    if missing("client").is_some() {
        return "The terminal's tmux client is gone.".to_string();
    }
    // `error connecting to /tmp/tmux-1000/default (Permission denied)`
    if let Some(rest) = line.strip_prefix("error connecting to ") {
        let reason = rest
            .rsplit_once(" (")
            .map(|(_, reason)| reason.trim_end_matches(')'))
            .unwrap_or("it refused");
        return format!("The tmux server can't be reached ({reason}).");
    }
    message.to_string()
}

/// The sentence for a pane id that names no pane.
pub fn no_pane(pane: &str) -> String {
    format!("There is no pane {pane}.")
}

/// The sentence for a session id or name that names no session.
pub fn no_session(reference: &str) -> String {
    if is_session_id(reference) {
        format!("There is no session {reference}.")
    } else {
        format!("There is no session named \"{reference}\".")
    }
}

/// The sentence for a window a session doesn't have.
pub fn no_window(session_id: &str, window: &WindowRef) -> String {
    match window {
        WindowRef::Id(id) => format!("Session {session_id} has no window {id}."),
        WindowRef::Index(index) => format!("Session {session_id} has no window at index {index}."),
    }
}

impl std::fmt::Display for TmuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoServer => f.write_str("no tmux server is running"),
            Self::Failed(message) => f.write_str(message),
            Self::Io(message) => write!(f, "could not run tmux: {message}"),
            Self::Timeout => f.write_str("tmux did not answer in time"),
        }
    }
}

/// tmux reports a missing server in a few shapes depending on whether the
/// socket file exists.
pub fn is_no_server(stderr: &str) -> bool {
    stderr.contains("no server running")
        || (stderr.contains("error connecting to")
            && (stderr.contains("No such file or directory")
                || stderr.contains("Connection refused")))
}

/// A resolved tmux binary bound to one server socket.
#[derive(Debug, Clone)]
pub struct Tmux {
    pub path: PathBuf,
    pub socket: Socket,
    pub timeout: Duration,
    /// As `-V` printed it (`3.6`, `next-3.7`); empty when not known.
    pub version: String,
}

impl Tmux {
    /// The binary plus socket arguments. `TMUX` is removed so a worker
    /// started from inside tmux neither talks to a different server than
    /// configured nor makes `attach` refuse to nest. `-u` because a daemon
    /// often has no UTF-8 locale, and then tmux prints [`SEP`] as `_` and no
    /// listing parses (simple-installs D29).
    pub fn command(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.path);
        command
            .arg("-u")
            .args(self.socket.args())
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        command
    }

    pub async fn run<I, S>(&self, args: I) -> Result<String, TmuxError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = self.command();
        command.args(args);
        let output = tokio::time::timeout(self.timeout, command.output())
            .await
            .map_err(|_| TmuxError::Timeout)?
            .map_err(|error| TmuxError::Io(error.to_string()))?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if is_no_server(&stderr) {
            return Err(TmuxError::NoServer);
        }
        Err(TmuxError::Failed(if stderr.is_empty() {
            format!("tmux exited with {}", output.status)
        } else {
            stderr
        }))
    }
}

impl Tmux {
    /// This tmux is `major.minor` or later. One whose version doesn't say
    /// (`next-3.7`, `master`, `openbsd-7.4`: built from tmux's own sources)
    /// counts as later.
    pub fn at_least(&self, major: u32, minor: u32) -> bool {
        version_at_least(&self.version, major, minor) != Some(false)
    }

    /// `name` as tmux takes it literally for a session (`new-session -s`,
    /// `rename-session`) or `rename-window`: tmux 2.7 and later expand formats
    /// there, so `#` is doubled on them and only on them.
    pub fn name_arg(&self, name: &str) -> String {
        if self.at_least(2, 7) {
            format_literal(name)
        } else {
            name.to_string()
        }
    }

    /// `name` as tmux takes it literally for a new window's `-n`
    /// (`new-session`, `new-window`), which tmux expands from 3.1 on.
    pub fn new_window_name_arg(&self, name: &str) -> String {
        if self.at_least(3, 1) {
            format_literal(name)
        } else {
            name.to_string()
        }
    }

    /// `#{pane_id}` then `fields` of pane `pane`, or the sentence that there
    /// is no such pane: from tmux 3.4 on, `display-message` answers for a
    /// target it can't find instead of failing.
    pub async fn pane_fields(&self, pane: &str, fields: &[&str]) -> Result<Vec<String>, String> {
        let format = std::iter::once("#{pane_id}")
            .chain(fields.iter().copied())
            .collect::<Vec<_>>()
            .join(&SEP.to_string());
        let output = self
            .run(["display-message", "-p", "-t", pane, format.as_str()])
            .await
            .map_err(|error| error.sentence())?;
        let line = unescape_separators(output.lines().next().unwrap_or_default());
        let values: Vec<String> = line
            .splitn(fields.len() + 1, SEP)
            .map(str::to_string)
            .collect();
        if values.first().map(String::as_str) != Some(pane) || values.len() != fields.len() + 1 {
            return Err(no_pane(pane));
        }
        Ok(values.into_iter().skip(1).collect())
    }

    /// Keeps new pane `pane` once its program ends (remain-on-exit on), then
    /// starts `command` in it. The pane was made running [`HOLD`] instead of
    /// `command`: a command that ends at once (a typo, a failing build) would
    /// otherwise be gone, pane and maybe window and session, before a second
    /// tmux command could set the option. Its own option from tmux 3.0; tmux
    /// 2.x has no pane options, so there it is its window's.
    pub async fn keep_when_done(&self, pane: &str, command: Option<&str>) -> Result<(), String> {
        let scope = if self.at_least(3, 0) { "-p" } else { "-w" };
        let mut args = vec!["set-option", scope, "-t", pane, "remain-on-exit", "on"];
        if let Some(command) = command {
            args.extend([";", "respawn-pane", "-k", "-t", pane, "--", command]);
        }
        let kept = self.run(&args).await.map(drop);
        if kept.is_err() {
            // Never leave a pane behind that runs the placeholder.
            let _ = self.run(["kill-pane", "-t", pane]).await;
        }
        kept.map_err(|error| error.sentence())
    }

    /// `(id, name)` of every session; empty when no server runs.
    pub async fn session_names(&self) -> Result<Vec<(String, String)>, TmuxError> {
        let format = format!("#{{session_id}}{SEP}#{{session_name}}");
        match self.run(["list-sessions", "-F", format.as_str()]).await {
            Ok(output) => Ok(output
                .lines()
                .filter_map(|line| {
                    let line = unescape_separators(line);
                    let (id, name) = line.split_once(SEP)?;
                    Some((id.to_string(), name.to_string()))
                })
                .collect()),
            Err(TmuxError::NoServer) => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    /// The id of the session named or numbered `reference` (exact match only).
    pub async fn resolve_session(&self, reference: &str) -> Result<String, String> {
        let reference = reference.trim();
        if reference.is_empty() {
            return Err("session must not be empty".to_string());
        }
        let sessions = self
            .session_names()
            .await
            .map_err(|error| error.sentence())?;
        find_session(&sessions, reference)
    }

    /// The id of `window` inside `session_id`.
    pub async fn resolve_window(
        &self,
        session_id: &str,
        window: &WindowRef,
    ) -> Result<String, String> {
        let windows = self.windows_of(session_id).await?;
        find_window(&windows, session_id, window).map(|found| found.id.clone())
    }

    /// The windows of `session_id`, in index order.
    pub async fn windows_of(&self, session_id: &str) -> Result<Vec<SessionWindow>, String> {
        let format = format!("#{{window_id}}{SEP}#{{window_index}}{SEP}#{{window_active}}");
        let output = self
            .run(["list-windows", "-t", session_id, "-F", format.as_str()])
            .await
            .map_err(|error| error.sentence())?;
        Ok(parse_session_windows(&output))
    }
}

/// The session `reference` names among `(id, name)` pairs. A session can be
/// named like an id (made outside this worker: its own names can't look like
/// one), so an id-shaped reference that is one session's id and another's
/// name is refused rather than guessed, and one that is only a name says
/// which id to use.
pub fn find_session(sessions: &[(String, String)], reference: &str) -> Result<String, String> {
    let named = sessions.iter().find(|(_, name)| name == reference);
    if !is_session_id(reference) {
        return named
            .map(|(id, _)| id.clone())
            .ok_or_else(|| no_session(reference));
    }
    let numbered = sessions.iter().find(|(id, _)| id == reference);
    match (numbered, named) {
        (Some((id, _)), Some((other, _))) if id != other => Err(format!(
            "\"{reference}\" is session {id}'s id and session {other}'s name; rename session {other} to tell them apart."
        )),
        (Some((id, _)), _) => Ok(id.clone()),
        (None, Some((other, _))) => Err(format!(
            "There is no session {reference}; the session named \"{reference}\" is {other}."
        )),
        (None, None) => Err(no_session(reference)),
    }
}

/// One window of a session as [`Tmux::windows_of`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionWindow {
    pub id: String,
    pub index: u32,
    /// The session's current window.
    pub active: bool,
}

pub fn parse_session_windows(output: &str) -> Vec<SessionWindow> {
    output
        .lines()
        .filter_map(|line| {
            let line = unescape_separators(line);
            let mut fields = line.split(SEP);
            let id = fields.next()?.to_string();
            let index = fields.next()?.trim().parse().ok()?;
            let active = fields.next().is_some_and(flag);
            Some(SessionWindow { id, index, active })
        })
        .collect()
}

/// `window` among the windows of `session_id`.
pub fn find_window<'a>(
    windows: &'a [SessionWindow],
    session_id: &str,
    window: &WindowRef,
) -> Result<&'a SessionWindow, String> {
    windows
        .iter()
        .find(|candidate| match window {
            WindowRef::Id(wanted) => &candidate.id == wanted,
            WindowRef::Index(wanted) => candidate.index == *wanted,
        })
        .ok_or_else(|| no_window(session_id, window))
}

/// Whether tmux `version` (as `-V` prints it: `3.6`, `3.1c`) is `major.minor`
/// or later. `None` when the version doesn't say (`next-3.7`, `master`,
/// `openbsd-7.4`): the caller tries and reads tmux's answer instead.
pub fn version_at_least(version: &str, major: u32, minor: u32) -> Option<bool> {
    let (have_major, rest) = version.trim().split_once('.')?;
    let have_major: u32 = have_major.parse().ok()?;
    let digits = rest.trim_end_matches(|c: char| c.is_ascii_lowercase());
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let have_minor: u32 = digits.parse().ok()?;
    Some((have_major, have_minor) >= (major, minor))
}

// ---------------------------------------------------------------------------
// Listing

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSession {
    pub id: String,
    pub name: String,
    pub created: u64,
    pub activity: u64,
    pub attached: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawWindow {
    pub session_id: String,
    pub id: String,
    pub index: u32,
    pub name: String,
    pub active: bool,
    pub activity: u64,
    pub bell: bool,
    pub zoomed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPane {
    pub session_id: String,
    pub window_id: String,
    pub id: String,
    pub index: u32,
    pub active: bool,
    pub pid: u32,
    pub command: String,
    pub cwd: String,
    pub width: u32,
    pub height: u32,
    pub dead: bool,
    /// `@iii_claude_account`: `global` or an account id (agent-accounts D4).
    pub claude_mark: String,
    /// `@iii_skip_permissions` is `1`.
    pub skip_mark: bool,
    /// `@iii_claude_pending`: the account the pane moves to after its turn.
    pub pending_mark: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawClient {
    pub tty: String,
    pub session_id: String,
    pub pid: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawServer {
    pub socket_path: String,
    pub pid: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawSnapshot {
    pub sessions: Vec<RawSession>,
    pub windows: Vec<RawWindow>,
    pub panes: Vec<RawPane>,
    pub clients: Vec<RawClient>,
    pub server: Option<RawServer>,
}

fn format(tag: char, fields: &[&str]) -> String {
    let mut out = String::from(tag);
    for field in fields {
        out.push(SEP);
        out.push_str(field);
    }
    out
}

/// One tmux invocation that lists sessions, windows, panes, clients and the
/// server, as `;`-separated commands.
pub fn list_all_args() -> Vec<String> {
    let session = format(
        'S',
        &[
            "#{session_id}",
            "#{session_name}",
            "#{session_created}",
            "#{session_activity}",
            "#{session_attached}",
        ],
    );
    let window = format(
        'W',
        &[
            "#{session_id}",
            "#{window_id}",
            "#{window_index}",
            "#{window_active}",
            "#{window_activity}",
            "#{window_bell_flag}",
            "#{window_zoomed_flag}",
            "#{window_name}",
        ],
    );
    let pane = format(
        'P',
        &[
            "#{session_id}",
            "#{window_id}",
            "#{pane_id}",
            "#{pane_index}",
            "#{pane_active}",
            "#{pane_pid}",
            "#{pane_width}",
            "#{pane_height}",
            "#{pane_dead}",
            "#{pane_current_command}",
            "#{pane_current_path}",
            // User options read through the pane: its own from tmux 3.0, its
            // window's before (agent-accounts D4).
            "#{@iii_claude_account}",
            "#{@iii_skip_permissions}",
            "#{@iii_claude_pending}",
            "#{pane_title}",
        ],
    );
    let client = format('C', &["#{client_tty}", "#{session_id}", "#{client_pid}"]);
    let server = format('X', &["#{socket_path}", "#{pid}"]);
    [
        vec!["list-sessions".into(), "-F".into(), session],
        vec![
            ";".into(),
            "list-windows".into(),
            "-a".into(),
            "-F".into(),
            window,
        ],
        vec![
            ";".into(),
            "list-panes".into(),
            "-a".into(),
            "-F".into(),
            pane,
        ],
        vec![";".into(), "list-clients".into(), "-F".into(), client],
        vec![";".into(), "display-message".into(), "-p".into(), server],
    ]
    .concat()
}

fn flag(value: &str) -> bool {
    value.trim() == "1"
}

fn number<T: std::str::FromStr + Default>(value: &str) -> T {
    value.trim().parse().unwrap_or_default()
}

pub fn parse_list_all(output: &str) -> RawSnapshot {
    let mut snapshot = RawSnapshot::default();
    for line in output.lines() {
        let line = unescape_separators(line);
        let mut chars = line.chars();
        let tag = chars.next();
        let Some(rest) = chars.as_str().strip_prefix(SEP) else {
            continue;
        };
        match tag {
            Some('S') => {
                let f: Vec<&str> = rest.splitn(5, SEP).collect();
                if f.len() == 5 {
                    snapshot.sessions.push(RawSession {
                        id: f[0].to_string(),
                        name: f[1].to_string(),
                        created: number(f[2]),
                        activity: number(f[3]),
                        attached: number(f[4]),
                    });
                }
            }
            Some('W') => {
                let f: Vec<&str> = rest.splitn(8, SEP).collect();
                if f.len() == 8 {
                    snapshot.windows.push(RawWindow {
                        session_id: f[0].to_string(),
                        id: f[1].to_string(),
                        index: number(f[2]),
                        active: flag(f[3]),
                        activity: number(f[4]),
                        bell: flag(f[5]),
                        zoomed: flag(f[6]),
                        name: f[7].to_string(),
                    });
                }
            }
            Some('P') => {
                let f: Vec<&str> = rest.splitn(15, SEP).collect();
                if f.len() == 15 {
                    snapshot.panes.push(RawPane {
                        session_id: f[0].to_string(),
                        window_id: f[1].to_string(),
                        id: f[2].to_string(),
                        index: number(f[3]),
                        active: flag(f[4]),
                        pid: number(f[5]),
                        width: number(f[6]),
                        height: number(f[7]),
                        dead: flag(f[8]),
                        command: f[9].to_string(),
                        cwd: f[10].to_string(),
                        claude_mark: f[11].trim().to_string(),
                        skip_mark: flag(f[12]),
                        pending_mark: f[13].trim().to_string(),
                        title: f[14].to_string(),
                    });
                }
            }
            Some('C') => {
                let f: Vec<&str> = rest.splitn(3, SEP).collect();
                if f.len() == 3 {
                    snapshot.clients.push(RawClient {
                        tty: f[0].to_string(),
                        session_id: f[1].to_string(),
                        pid: number(f[2]),
                    });
                }
            }
            Some('X') => {
                let f: Vec<&str> = rest.splitn(2, SEP).collect();
                if f.len() == 2 {
                    snapshot.server = Some(RawServer {
                        socket_path: f[0].to_string(),
                        pid: number(f[1]),
                    });
                }
            }
            _ => {}
        }
    }
    snapshot
}

// ---------------------------------------------------------------------------
// Names and targets

/// Session and window names as tmux keeps them: tmux rewrites `:` and `.`
/// (target separators) itself, so they are refused instead of silently
/// changing the name the caller asked for. A name shaped like an id (`$2`,
/// `@3`, `%4`) is refused too: callers name sessions by id or name, and such
/// a name would read as another session's id.
pub fn validate_name(kind: &str, name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err(format!("{kind} name must not be empty"));
    }
    if name.chars().count() > 64 {
        return Err(format!("{kind} name must be at most 64 characters"));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(format!("{kind} name must not contain control characters"));
    }
    if name.contains([':', '.']) {
        return Err(format!("{kind} name must not contain ':' or '.'"));
    }
    if looks_like_id(name.trim()) {
        return Err(format!(
            "A {kind} name can't look like a tmux id such as $2, @3 or %4."
        ));
    }
    Ok(())
}

/// `$N`, `@N` or `%N`: a session, window or pane id.
pub fn looks_like_id(value: &str) -> bool {
    is_session_id(value) || is_window_id(value) || is_pane_id(value)
}

pub fn is_session_id(value: &str) -> bool {
    value
        .strip_prefix('$')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

pub fn is_window_id(value: &str) -> bool {
    value
        .strip_prefix('@')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

pub fn is_pane_id(value: &str) -> bool {
    value
        .strip_prefix('%')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// A window as callers name it: an id (`"@3"`) or an index (`2` or `"2"`).
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize, schemars::JsonSchema,
)]
#[serde(untagged)]
pub enum WindowArg {
    /// Window index inside the session.
    Index(u32),
    /// Window id like `@3`, or an index as text.
    Text(String),
}

/// A window by id (`@3`) or by index inside its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowRef {
    Id(String),
    Index(u32),
}

impl WindowRef {
    pub fn parse(raw: &WindowArg) -> Result<Self, String> {
        match raw {
            WindowArg::Index(index) => Ok(Self::Index(*index)),
            WindowArg::Text(text) => {
                let text = text.trim();
                if is_window_id(text) {
                    Ok(Self::Id(text.to_string()))
                } else {
                    text.parse().map(Self::Index).map_err(|_| {
                        format!("window must be an id like @3 or an index, got '{text}'")
                    })
                }
            }
        }
    }
}

pub fn window_target(session_id: &str, window: &WindowRef) -> String {
    match window {
        WindowRef::Id(id) => format!("{session_id}:{id}"),
        WindowRef::Index(index) => format!("{session_id}:{index}"),
    }
}

/// `text` as tmux reads it back literally where it expands formats, as in
/// every `-c` start directory and (by version, see [`Tmux::name_arg`]) in
/// names: `#{…}` would be replaced and `#(…)` would run a shell command, so
/// each `#` is doubled.
pub fn format_literal(text: &str) -> String {
    text.replace('#', "##")
}

/// Expand `~/`, require an absolute existing directory.
pub fn resolve_cwd(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let expanded = if raw == "~" || raw.starts_with("~/") {
        let home = std::env::var("HOME")
            .map_err(|_| "HOME is not set; use an absolute cwd".to_string())?;
        format!("{home}{}", &raw[1..])
    } else {
        raw.to_string()
    };
    let path = std::path::Path::new(&expanded);
    if !path.is_absolute() {
        return Err(format!("cwd must be an absolute path, got '{raw}'"));
    }
    if !path.is_dir() {
        return Err(format!("cwd '{expanded}' is not an existing directory"));
    }
    Ok(expanded)
}

pub fn validate_command(command: &str) -> Result<(), String> {
    if command.trim().is_empty() {
        return Err("command must not be empty when given".to_string());
    }
    if command.contains('\0') {
        return Err("command must not contain NUL bytes".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_asks_for_utf8_before_naming_its_socket() {
        let tmux = Tmux {
            path: PathBuf::from("/x/tmux"),
            socket: Socket::Name("work".to_string()),
            timeout: Duration::from_secs(1),
            version: "3.6".to_string(),
        };
        let command = tmux.command();
        let args: Vec<_> = command.as_std().get_args().collect();
        assert_eq!(args, ["-u", "-L", "work"]);
    }

    fn line(tag: char, fields: &[&str]) -> String {
        let mut out = String::from(tag);
        for field in fields {
            out.push(SEP);
            out.push_str(field);
        }
        out
    }

    #[test]
    fn parses_a_full_listing() {
        let output = [
            line('S', &["$1", "my sess", "1791592553", "1791592600", "2"]),
            line(
                'W',
                &["$1", "@4", "1", "1", "1791592600", "0", "1", "claude code"],
            ),
            line(
                'P',
                &[
                    "$1",
                    "@4",
                    "%7",
                    "0",
                    "1",
                    "4242",
                    "147",
                    "56",
                    "0",
                    "claude",
                    "/home/u/ação",
                    "personal",
                    "1",
                    "",
                    "✳ Tarefa: a\u{1f}b",
                ],
            ),
            line('C', &["/dev/pts/3", "$1", "999"]),
            line('X', &["/tmp/tmux-1000/default", "41901"]),
            "garbage".to_string(),
        ]
        .join("\n");
        let snapshot = parse_list_all(&output);
        assert_eq!(snapshot.sessions[0].name, "my sess");
        assert_eq!(snapshot.sessions[0].attached, 2);
        assert_eq!(snapshot.windows[0].name, "claude code");
        assert!(snapshot.windows[0].zoomed && !snapshot.windows[0].bell);
        let pane = &snapshot.panes[0];
        assert_eq!(pane.pid, 4242);
        assert_eq!(pane.cwd, "/home/u/ação");
        assert_eq!(
            (
                pane.claude_mark.as_str(),
                pane.skip_mark,
                pane.pending_mark.as_str()
            ),
            ("personal", true, "")
        );
        assert_eq!(
            pane.title, "✳ Tarefa: a\u{1f}b",
            "the title keeps a stray separator"
        );
        assert_eq!(snapshot.clients[0].tty, "/dev/pts/3");
        assert_eq!(snapshot.server.unwrap().pid, 41901);
    }

    #[test]
    fn escaped_separators_parse_like_raw_ones() {
        // Ubuntu's tmux 3.4 prints U+001F as `\037`; Unicode and backslashes
        // stay as they are.
        let output = [
            line('S', &["$1", "my sess", "1791592553", "1791592600", "2"]),
            line(
                'P',
                &[
                    "$1",
                    "@4",
                    "%7",
                    "0",
                    "1",
                    "4242",
                    "147",
                    "56",
                    "0",
                    "claude",
                    "/home/u/ação",
                    "",
                    "",
                    "",
                    "a\\b",
                ],
            ),
        ]
        .join("\n")
        .replace(SEP, ESCAPED_SEP);
        let snapshot = parse_list_all(&output);
        assert_eq!(snapshot.sessions[0].name, "my sess");
        assert_eq!(snapshot.sessions[0].attached, 2);
        let pane = &snapshot.panes[0];
        assert_eq!((pane.id.as_str(), pane.pid), ("%7", 4242));
        assert_eq!(pane.cwd, "/home/u/ação");
        assert_eq!(pane.title, "a\\b");

        let raw = format!("$1{SEP}a\\037b");
        assert_eq!(unescape_separators(&raw), raw, "a raw separator wins");
        assert_eq!(unescape_separators("plain"), "plain");
    }

    #[test]
    fn list_all_chains_commands_with_semicolons() {
        let args = list_all_args();
        assert_eq!(args.iter().filter(|arg| *arg == ";").count(), 4);
        assert_eq!(args[0], "list-sessions");
    }

    #[test]
    fn names_refuse_target_separators() {
        validate_name("session", "my sess").unwrap();
        validate_name("session", "ação-1").unwrap();
        assert!(validate_name("session", "a:b").is_err());
        assert!(validate_name("session", "a.b").is_err());
        assert!(validate_name("session", " ").is_err());
        assert!(validate_name("session", "a\nb").is_err());
        assert!(validate_name("session", &"x".repeat(65)).is_err());
    }

    #[test]
    fn names_that_look_like_ids_are_refused() {
        for name in ["$2", "@3", "%4", " $12 "] {
            assert_eq!(
                validate_name("session", name).unwrap_err(),
                "A session name can't look like a tmux id such as $2, @3 or %4."
            );
        }
        assert_eq!(
            validate_name("window", "@0").unwrap_err(),
            "A window name can't look like a tmux id such as $2, @3 or %4."
        );
        for name in ["$", "$2a", "a$2", "@x", "%", "100%", "$ 2"] {
            validate_name("session", name).unwrap();
        }
    }

    #[test]
    fn a_session_is_found_by_id_or_by_name_never_by_a_guess() {
        let sessions: Vec<(String, String)> = [("$0", "work"), ("$2", "$5"), ("$5", "other")]
            .iter()
            .map(|(id, name)| (id.to_string(), name.to_string()))
            .collect();
        assert_eq!(find_session(&sessions, "work").unwrap(), "$0");
        assert_eq!(find_session(&sessions, "$0").unwrap(), "$0");
        assert_eq!(find_session(&sessions, "$2").unwrap(), "$2");
        // `$5` is session $5's id and session $2's name.
        assert_eq!(
            find_session(&sessions, "$5").unwrap_err(),
            "\"$5\" is session $5's id and session $2's name; rename session $2 to tell them apart."
        );
        let renamed: Vec<(String, String)> = vec![("$2".into(), "$9".into())];
        assert_eq!(
            find_session(&renamed, "$9").unwrap_err(),
            "There is no session $9; the session named \"$9\" is $2."
        );
        assert_eq!(
            find_session(&sessions, "$7").unwrap_err(),
            "There is no session $7."
        );
        assert_eq!(
            find_session(&sessions, "nope").unwrap_err(),
            "There is no session named \"nope\"."
        );
        // A name that is an id's name of itself is fine.
        let same: Vec<(String, String)> = vec![("$3".into(), "$3".into())];
        assert_eq!(find_session(&same, "$3").unwrap(), "$3");
    }

    #[test]
    fn tmux_refusals_are_one_sentence() {
        let failed = |message: &str| TmuxError::Failed(message.into()).sentence();
        // tmux 3.0 and later, then 2.6.
        for (message, sentence) in [
            ("can't find pane: %9", "There is no pane %9."),
            ("can't find pane %9", "There is no pane %9."),
            ("can't find window: @3", "There is no window @3."),
            ("can't find window @3", "There is no window @3."),
            ("can't find session: $4", "There is no session $4."),
            (
                "can't find session nope",
                "There is no session named \"nope\".",
            ),
            (
                "error connecting to /tmp/tmux-1000/default (Permission denied)",
                "The tmux server can't be reached (Permission denied).",
            ),
            (
                "can't find client: /dev/pts/3",
                "The terminal's tmux client is gone.",
            ),
            (
                "can't find client /dev/ttys003",
                "The terminal's tmux client is gone.",
            ),
            ("index in use: 3", "index in use: 3"),
        ] {
            assert_eq!(failed(message), sentence, "{message}");
        }
        assert_eq!(TmuxError::NoServer.sentence(), "No tmux server is running.");
        assert_eq!(
            TmuxError::Timeout.sentence(),
            "tmux did not answer in time."
        );
        assert!(!failed("error connecting to /tmp/x/y (Permission denied)").contains("/tmp"));
    }

    #[test]
    fn names_are_escaped_only_where_tmux_expands_them() {
        let tmux = |version: &str| Tmux {
            path: PathBuf::from("/x/tmux"),
            socket: Socket::Default,
            timeout: Duration::from_secs(1),
            version: version.to_string(),
        };
        // Checked against tmux 2.6, 3.0a, 3.1c, 3.4 and 3.8.
        assert_eq!(tmux("2.6").name_arg("a#b"), "a#b");
        assert_eq!(tmux("2.7").name_arg("a#b"), "a##b");
        assert_eq!(tmux("3.0a").new_window_name_arg("a#b"), "a#b");
        assert_eq!(tmux("3.1c").new_window_name_arg("a#{x}"), "a##{x}");
        assert_eq!(tmux("next-3.7").name_arg("#"), "##");
        assert_eq!(tmux("").new_window_name_arg("#"), "##");
        assert!(tmux("3.2").at_least(3, 2) && !tmux("3.1c").at_least(3, 2));
    }

    #[test]
    fn window_refs_accept_ids_and_indexes() {
        let arg = |value: serde_json::Value| serde_json::from_value::<WindowArg>(value).unwrap();
        assert_eq!(
            WindowRef::parse(&arg(serde_json::json!("@3"))).unwrap(),
            WindowRef::Id("@3".into())
        );
        assert_eq!(
            WindowRef::parse(&arg(serde_json::json!("2"))).unwrap(),
            WindowRef::Index(2)
        );
        assert_eq!(
            WindowRef::parse(&arg(serde_json::json!(4))).unwrap(),
            WindowRef::Index(4)
        );
        assert!(WindowRef::parse(&arg(serde_json::json!("@x"))).is_err());
        assert!(serde_json::from_value::<WindowArg>(serde_json::json!(-1)).is_err());
        assert_eq!(window_target("$1", &WindowRef::Index(2)), "$1:2");
        assert_eq!(window_target("$1", &WindowRef::Id("@9".into())), "$1:@9");
    }

    #[test]
    fn session_windows_parse_and_resolve() {
        let output = [
            ["@1", "0", "0"].join(&SEP.to_string()),
            ["@4", "2", "1"].join(&SEP.to_string()),
            "@9\\0375\\0370".to_string(),
            "garbage".to_string(),
        ]
        .join("\n");
        let windows = parse_session_windows(&output);
        assert_eq!(
            windows,
            vec![
                SessionWindow {
                    id: "@1".into(),
                    index: 0,
                    active: false
                },
                SessionWindow {
                    id: "@4".into(),
                    index: 2,
                    active: true
                },
                SessionWindow {
                    id: "@9".into(),
                    index: 5,
                    active: false
                },
            ]
        );
        assert_eq!(
            find_window(&windows, "$1", &WindowRef::Index(2))
                .unwrap()
                .id,
            "@4"
        );
        assert_eq!(
            find_window(&windows, "$1", &WindowRef::Id("@9".into()))
                .unwrap()
                .index,
            5
        );
        assert_eq!(
            find_window(&windows, "$1", &WindowRef::Index(1)).unwrap_err(),
            "Session $1 has no window at index 1."
        );
        assert_eq!(
            find_window(&windows, "$1", &WindowRef::Id("@2".into())).unwrap_err(),
            "Session $1 has no window @2."
        );
    }

    #[test]
    fn versions_compare_by_major_and_minor() {
        assert_eq!(version_at_least("3.2", 3, 2), Some(true));
        assert_eq!(version_at_least("3.2a", 3, 2), Some(true));
        assert_eq!(version_at_least("3.1c", 3, 2), Some(false));
        assert_eq!(version_at_least("3.10", 3, 2), Some(true));
        assert_eq!(version_at_least("4.0", 3, 2), Some(true));
        assert_eq!(version_at_least("2.1", 2, 1), Some(true));
        assert_eq!(version_at_least("2.0", 2, 1), Some(false));
        assert_eq!(version_at_least("1.8", 2, 1), Some(false));
        assert_eq!(version_at_least("next-3.7", 3, 2), None);
        assert_eq!(version_at_least("master", 3, 2), None);
        assert_eq!(version_at_least("openbsd-7.4", 3, 2), None);
        assert_eq!(version_at_least("3.", 3, 2), None);
    }

    #[test]
    fn ids_are_recognised() {
        assert!(is_session_id("$12"));
        assert!(!is_session_id("$"));
        assert!(!is_session_id("12"));
        assert!(is_window_id("@0"));
        assert!(!is_window_id("@a"));
        assert!(is_pane_id("%5"));
        assert!(!is_pane_id("%"));
        assert!(!is_pane_id("5"));
        assert!(!is_pane_id("%5;kill-server"));
    }

    #[test]
    fn missing_server_messages_are_recognised() {
        assert!(is_no_server("no server running on /tmp/tmux-1000/default"));
        assert!(is_no_server(
            "error connecting to /tmp/tmux-1000/x (No such file or directory)"
        ));
        assert!(!is_no_server("can't find session: nope"));
    }

    #[test]
    fn format_text_is_doubled_hashes() {
        assert_eq!(format_literal("/home/u/a#b"), "/home/u/a##b");
        assert_eq!(format_literal("/x/#{session_name}"), "/x/##{session_name}");
        assert_eq!(format_literal("/x/#(id)"), "/x/##(id)");
        assert_eq!(format_literal("/plain"), "/plain");
    }

    #[test]
    fn cwd_must_be_an_existing_absolute_directory() {
        assert!(resolve_cwd("relative/dir").is_err());
        assert!(resolve_cwd("/definitely/not/here").is_err());
        assert_eq!(resolve_cwd("/").unwrap(), "/");
    }
}
