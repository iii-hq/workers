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
}

impl Tmux {
    /// The binary plus socket arguments. `TMUX` is removed so a worker
    /// started from inside tmux neither talks to a different server than
    /// configured nor makes `attach` refuse to nest.
    pub fn command(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.path);
        command
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
            .map_err(|error| error.to_string())?;
        sessions
            .into_iter()
            .find(|(id, name)| {
                if is_session_id(reference) {
                    id == reference
                } else {
                    name == reference
                }
            })
            .map(|(id, _)| id)
            .ok_or_else(|| format!("session \"{reference}\" not found"))
    }

    /// The id of `window` inside `session_id`.
    pub async fn resolve_window(
        &self,
        session_id: &str,
        window: &WindowRef,
    ) -> Result<String, String> {
        let format = format!("#{{window_id}}{SEP}#{{window_index}}");
        let output = self
            .run(["list-windows", "-t", session_id, "-F", format.as_str()])
            .await
            .map_err(|error| error.to_string())?;
        output
            .lines()
            .filter_map(|line| {
                let line = unescape_separators(line);
                let (id, index) = line.split_once(SEP)?;
                Some((id.to_string(), index.to_string()))
            })
            .find(|(id, index)| match window {
                WindowRef::Id(wanted) => id == wanted,
                WindowRef::Index(wanted) => index.parse::<u32>().ok() == Some(*wanted),
            })
            .map(|(id, _)| id)
            .ok_or_else(|| match window {
                WindowRef::Id(id) => format!("window {id} not found in session {session_id}"),
                WindowRef::Index(index) => {
                    format!("window {index} not found in session {session_id}")
                }
            })
    }
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
                let f: Vec<&str> = rest.splitn(12, SEP).collect();
                if f.len() == 12 {
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
                        title: f[11].to_string(),
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
/// changing the name the caller asked for.
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
    Ok(())
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
    fn cwd_must_be_an_existing_absolute_directory() {
        assert!(resolve_cwd("relative/dir").is_err());
        assert!(resolve_cwd("/definitely/not/here").is_err());
        assert_eq!(resolve_cwd("/").unwrap(), "/");
    }
}
