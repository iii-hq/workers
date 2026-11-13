//! Signing a Claude Code account in from the app (claude-login-ui WD1–WD12):
//! `claude auth login` in a hidden pty, no tmux window.
//!
//! [`Logins::start`] runs Claude Code on the account's folder (or on the slot,
//! `~/.claude`, for the global account) in a pty 2000 columns wide, reads the
//! sign-in link it prints and answers with it. Claude Code then waits for the
//! code the sign-in page shows; [`Logins::code`] types it in and waits for
//! the outcome. Claude Code also hands a second link, whose return to its
//! loopback listener finishes the sign-in with no code, to `$BROWSER` (else
//! `open` on macOS, `xdg-open` on Linux): the worker sets `BROWSER` to a shim
//! of its own that notes that link and, unless the browser is to open on this
//! computer ([`LoginBrowser::Host`]), opens nothing.
//!
//! A login lasts 10 minutes ([`LINK_TTL_MS`]); then Claude Code is stopped.
//! The live feed lists every login under way and those that ended in the
//! last minute. Neither the code nor anything Claude Code prints after it is
//! logged or kept once the login is over; the link is logged without its
//! `state`, `code_challenge` and `login_hint` values ([`redact_url`]).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::{watch, Notify};

use super::launch::shell_quote;
use super::Accounts;
use crate::model::{now_ms, ClaudeLogin, LoginBrowser, LoginError, LoginState};

/// How long a sign-in link works: then Claude Code is stopped (`expired`).
pub const LINK_TTL_MS: u64 = 10 * 60_000;
/// How long a login that ended stays in the feed.
pub const LINGER_MS: u64 = 60_000;
/// How long `login_code` waits for Claude Code to check a code.
pub const CODE_WAIT: Duration = Duration::from_secs(60);
/// How long Claude Code may take to print its link.
pub const URL_WAIT: Duration = Duration::from_secs(45);
/// How long, with the browser on this computer, Claude Code may take to
/// hand it its link once it printed the other.
const BROWSER_NOTE_WAIT: Duration = Duration::from_secs(3);
/// Wide enough for any link on one line (Claude Code doesn't wrap, but a
/// pty this wide also keeps anything that would from wrapping it).
const COLS: u16 = 2000;
const ROWS: u16 = 50;
/// What is kept of Claude Code's output; it prints a few lines.
const OUTPUT_CAP: usize = 1 << 20;
/// The longest code taken: macOS reads at most 1024 bytes in a line.
const CODE_MAX: usize = 1000;

/// What Claude Code (2.1.323) says when Anthropic refuses a pasted code
/// (`invalid_grant`); it then ends with status 1 (WD6).
const REFUSED: [&str; 2] = ["That code wasn't accepted", "Invalid authorization code"];
/// What Claude Code says to a line without `#`, and keeps waiting (WD5).
const SHAPELESS: &str = "Invalid code. Please make sure the full code was copied.";

/// Environment Claude Code must not inherit from the worker: tmux's, the
/// engine's, a Claude Code the worker would run inside, and a refresh token
/// that would make `claude auth login` sign in with no link at all.
const STRIPPED_ENV: [&str; 9] = [
    "TMUX",
    "TMUX_PANE",
    "III_WORKER_NAME",
    "III_NAMESPACE",
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
    "CLAUDE_CODE_OAUTH_SCOPES",
    "BROWSER",
];

// ---------------------------------------------------------------- pure parts

/// The code as pasted, all whitespace taken out, when it has the shape
/// Claude Code takes: `<code>#<state>`, both halves non-empty, printable
/// ASCII, at most [`CODE_MAX`] characters (WD5).
pub fn clean_code(raw: &str) -> Option<String> {
    let code: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if code.is_empty() || code.len() > CODE_MAX || !code.chars().all(|c| c.is_ascii_graphic()) {
        return None;
    }
    let mut halves = code.split('#');
    let first = halves.next().unwrap_or_default();
    let second = halves.next().unwrap_or_default();
    (!first.is_empty() && !second.is_empty()).then_some(code)
}

fn is_sign_in_url(url: &str) -> bool {
    url.starts_with("https://") && url.contains("/oauth/authorize?")
}

/// The targets of the OSC 8 links in `text` that are complete
/// (`ESC ]8;<params>;<uri>` ended by BEL or `ESC \`).
fn osc8_targets(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("\u{1b}]8;") {
        rest = &rest[start + 4..];
        let Some(params_end) = rest.find(';') else {
            break;
        };
        let after = &rest[params_end + 1..];
        let Some(end) = after.find(['\u{7}', '\u{1b}']) else {
            break;
        };
        if end > 0 {
            found.push(&after[..end]);
        }
        rest = &after[end..];
    }
    found
}

/// `text` without terminal escapes: CSI sequences, OSC strings (ended by
/// BEL or `ESC \`) and two-byte escapes.
pub fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                // Parameters, then one final byte in @..~.
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' {
                        chars.next_if_eq(&'\\');
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The sign-in link in what Claude Code printed so far (WD4): the first
/// complete OSC 8 link to `…/oauth/authorize?…`, else the first such URL
/// in the plain text that a space or line end follows (so a link still
/// arriving isn't taken half).
pub fn find_url(output: &str) -> Option<String> {
    if let Some(url) = osc8_targets(output).into_iter().find(|t| is_sign_in_url(t)) {
        return Some(url.to_string());
    }
    let plain = strip_escapes(output);
    let complete = &plain[..plain.rfind(char::is_whitespace).unwrap_or(0)];
    complete
        .split_whitespace()
        .find(|word| is_sign_in_url(word))
        .map(str::to_string)
}

/// `url` with the values of `state`, `code_challenge` and `login_hint`
/// replaced by `…`, for logs.
pub fn redact_url(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let query: Vec<String> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((key, _)) if matches!(key, "state" | "code_challenge" | "login_hint") => {
                format!("{key}=…")
            }
            _ => pair.to_string(),
        })
        .collect();
    format!("{base}?{}", query.join("&"))
}

/// How Claude Code ended a login it was given a code for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Anthropic refused the code: Claude Code's sentence.
    Rejected(String),
    /// Anything else: Claude Code's sentence, or ours.
    Failed(String),
}

/// What Claude Code said after "Login failed: " in `output`, without
/// escapes and never with `secret` (the code, which the pty echoed) in it.
fn failure_line(output: &str, secret: Option<&str>) -> Option<String> {
    let plain = strip_escapes(output);
    let line = plain
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("Login failed:"))?
        .trim();
    let line = match secret.filter(|s| !s.is_empty()) {
        Some(secret) => line.replace(secret, "…"),
        None => line.to_string(),
    };
    (!line.is_empty()).then(|| line.chars().take(300).collect())
}

/// The outcome of a login from its exit (`success`) and what Claude Code
/// printed after the code was typed (WD6).
pub fn outcome(success: bool, output: &str, code: Option<&str>) -> Outcome {
    if success {
        return Outcome::Done;
    }
    match failure_line(output, code) {
        Some(line) if REFUSED.iter().any(|refused| line.contains(refused)) => {
            Outcome::Rejected(line)
        }
        Some(line) => Outcome::Failed(line),
        None => Outcome::Failed("Claude Code ended without signing in.".to_string()),
    }
}

/// Why Claude Code ended before it printed a link: its "Login failed" or
/// "Error:" line, else its last line (nothing secret is typed in yet).
fn early_end(output: &str, status: Option<i32>) -> String {
    if let Some(line) = failure_line(output, None) {
        return format!("Claude Code could not start the sign-in: {line}");
    }
    let plain = strip_escapes(output);
    let last = plain
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(300).collect::<String>());
    match (last, status) {
        (Some(line), _) => format!("Claude Code ended before it showed a sign-in link: {line}"),
        (None, Some(code)) => {
            format!("Claude Code ended before it showed a sign-in link (exit status {code}).")
        }
        (None, None) => "Claude Code ended before it showed a sign-in link.".to_string(),
    }
}

/// The `BROWSER` Claude Code is given (WD2): a shell script that notes the
/// link it was handed in `note` (whole: written aside, then renamed) and
/// then runs `opener` with it, or opens nothing.
pub fn browser_shim(note: &Path, opener: Option<&Path>) -> String {
    let note = shell_quote(&note.to_string_lossy());
    let mut script = format!(
        "#!/bin/sh\n# The iii tmux worker's sign-in: notes the link Claude Code opens.\n\
         printf '%s\\n' \"$1\" > {note}.part && mv -f {note}.part {note}\n"
    );
    match opener {
        Some(opener) => {
            script.push_str(&format!(
                "exec {} \"$1\"\n",
                shell_quote(&opener.to_string_lossy())
            ));
        }
        None => script.push_str("exit 0\n"),
    }
    script
}

fn login_error(code: &str, message: impl Into<String>) -> LoginError {
    LoginError {
        code: code.to_string(),
        message: message.into(),
    }
}

/// `{code, message}` as one JSON object, the handler error the apps read.
pub fn to_wire(error: &LoginError) -> String {
    serde_json::json!({"code": error.code, "message": error.message}).to_string()
}

pub fn gone() -> LoginError {
    login_error(
        "gone",
        "This sign-in is over: get a new link (tmux::accounts::login_start).",
    )
}

fn expired() -> LoginError {
    login_error(
        "expired",
        "The sign-in link expired after 10 minutes: get a new one.",
    )
}

pub fn invalid_code() -> LoginError {
    login_error(
        "invalid_code",
        "That isn't a sign-in code: on the sign-in page, copy the code (it has a # in the middle).",
    )
}

// ------------------------------------------------------------- the runtime

/// How logins run. The defaults are the real ones; tests name a fake
/// Claude Code, a trap for the browser and a shorter link.
#[derive(Debug, Clone, Default)]
pub struct Runner {
    /// The `claude` to run; by default the one on `PATH` or in the usual
    /// folders.
    pub claude: Option<PathBuf>,
    /// Added to its environment.
    pub env: Vec<(String, String)>,
    /// What opens a link for `browser: host`; by default the worker's
    /// `BROWSER`, else `open` (macOS) or `xdg-open` (Linux).
    pub opener: Option<PathBuf>,
    /// How long a link works, instead of [`LINK_TTL_MS`].
    pub ttl_ms: Option<u64>,
}

/// Where and how a login runs, decided under the swap gate.
#[derive(Debug, Clone)]
pub struct Plan {
    pub account: String,
    /// The account's folder, or `None` for the slot (`CLAUDE_CONFIG_DIR`
    /// unset).
    pub config_dir: Option<PathBuf>,
    /// Claude Code's home folder (the worker's).
    pub home: PathBuf,
    /// For `--email`.
    pub email: Option<String>,
    pub browser: LoginBrowser,
}

/// Where a login is, as its waiters see it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Status {
    state: LoginState,
    error: Option<LoginError>,
    ended_at_ms: Option<u64>,
    /// With `done`: the account the sign-in landed on (on the slot, the
    /// global account once a sign-in to another one was taken in, G6).
    signed_in_as: Option<String>,
    /// Lines Claude Code refused for their shape since the start.
    shapeless: u32,
}

/// What Claude Code printed: kept in memory only, dropped with the login.
#[derive(Default)]
struct Output {
    bytes: Mutex<Vec<u8>>,
    changed: Notify,
}

impl Output {
    fn push(&self, chunk: &[u8]) {
        let mut bytes = self.bytes.lock().unwrap_or_else(|p| p.into_inner());
        let room = OUTPUT_CAP.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
        drop(bytes);
        self.changed.notify_one();
    }

    fn len(&self) -> usize {
        self.bytes.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    fn text_from(&self, at: usize) -> String {
        let bytes = self.bytes.lock().unwrap_or_else(|p| p.into_inner());
        String::from_utf8_lossy(&bytes[at.min(bytes.len())..]).into_owned()
    }
}

struct Pty {
    /// Kept open: closing it hangs Claude Code up.
    _master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
}

struct Login {
    id: String,
    account: String,
    on_slot: bool,
    url: String,
    browser: LoginBrowser,
    email: Option<String>,
    started_at_ms: u64,
    expires_at_ms: u64,
    /// The shims and the link they note: removed when the login ends.
    folder: PathBuf,
    pid: Option<u32>,
    status: watch::Sender<Status>,
    /// The exit status once Claude Code ended (`None` inside: unknown).
    exit: watch::Receiver<Option<Option<i32>>>,
    pty: Mutex<Option<Pty>>,
    output: Arc<Output>,
    /// Where the output stood when the latest code was typed, and the code
    /// (to keep it out of any message).
    typed: Mutex<Option<(usize, String)>>,
    stop: Notify,
    browser_link: Mutex<Option<String>>,
}

impl Login {
    fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    fn ended(&self) -> bool {
        self.status.borrow().ended_at_ms.is_some()
    }

    /// Ends it with `state` and `error` unless it ended already; returns
    /// whether this did.
    fn end(
        &self,
        state: LoginState,
        error: Option<LoginError>,
        signed_in_as: Option<String>,
    ) -> bool {
        let mut ended = false;
        self.status.send_if_modified(|status| {
            if status.ended_at_ms.is_some() {
                return false;
            }
            status.state = state;
            status.error = error;
            status.signed_in_as = signed_in_as;
            status.ended_at_ms = Some(now_ms());
            ended = true;
            true
        });
        if ended {
            // Nothing Claude Code printed, the code included, outlives it.
            self.output
                .bytes
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clear();
            *self.typed.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
        ended
    }

    /// Stop Claude Code (its process group: it is the pty's session
    /// leader) unless it ended already.
    fn kill(&self) {
        if self.exit.borrow().is_none() {
            if let Some(pid) = self.pid.and_then(|pid| i32::try_from(pid).ok()) {
                // SAFETY: kill has no memory preconditions; a negative pid
                // names the process group Claude Code leads (the pty's
                // session), not yet reaped, so not reused.
                unsafe {
                    libc::kill(-pid, libc::SIGTERM);
                }
            }
        }
        // Closing the pty hangs up whatever is left.
        *self.pty.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// With the browser on this computer: the link Claude Code gave it,
    /// once the shim noted it.
    fn browser_url(&self) -> Option<String> {
        if self.browser != LoginBrowser::Host {
            return None;
        }
        let mut known = self.browser_link.lock().unwrap_or_else(|p| p.into_inner());
        if known.is_none() {
            *known = std::fs::read_to_string(self.folder.join("browser-link"))
                .ok()
                .map(|text| text.trim().to_string())
                .filter(|url| url.starts_with("http://") || url.starts_with("https://"));
        }
        known.clone()
    }

    fn entry(&self) -> ClaudeLogin {
        let status = self.status();
        ClaudeLogin {
            login_id: self.id.clone(),
            account: self.account.clone(),
            state: status.state,
            url: self.url.clone(),
            browser_url: if self.ended() {
                None
            } else {
                self.browser_url()
            },
            expires_at_ms: self.expires_at_ms,
            browser: self.browser,
            email: self.email.clone(),
            error: status.error,
        }
    }
}

/// The sign-ins under way and those that ended in the last minute.
#[derive(Default)]
pub struct Logins {
    logins: Mutex<HashMap<String, Arc<Login>>>,
    runner: Mutex<Runner>,
}

/// What the feed is told after a change it can't see by itself.
pub type Poke = Arc<dyn Fn() + Send + Sync>;

impl Logins {
    /// Run logins with `runner` (tests).
    pub fn set_runner(&self, runner: Runner) {
        *self.runner.lock().unwrap_or_else(|p| p.into_inner()) = runner;
    }

    fn get(&self, id: &str) -> Option<Arc<Login>> {
        self.logins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }

    /// The feed's `logins`: under way, and ended less than a minute ago;
    /// older ones are dropped.
    pub fn entries(&self, now: u64) -> Vec<ClaudeLogin> {
        let mut logins = self.logins.lock().unwrap_or_else(|p| p.into_inner());
        logins.retain(|_, login| {
            login
                .status()
                .ended_at_ms
                .is_none_or(|ended| now.saturating_sub(ended) < LINGER_MS)
        });
        let mut list: Vec<&Arc<Login>> = logins.values().collect();
        list.sort_by(|a, b| a.started_at_ms.cmp(&b.started_at_ms).then(a.id.cmp(&b.id)));
        list.into_iter().map(|login| login.entry()).collect()
    }

    /// Whether a login under way runs on account `id`'s place.
    pub fn running_for(&self, id: &str) -> bool {
        self.logins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .any(|login| login.account == id && !login.ended())
    }

    /// Stop login `id`: it leaves the list at once; a `login_code` waiting
    /// on it answers `gone`. Unknown or over: nothing to do.
    pub fn cancel(&self, id: &str) -> bool {
        let removed = self
            .logins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
        let Some(login) = removed else {
            return false;
        };
        let cancelled = login.end(
            LoginState::Failed,
            Some(login_error("gone", "The sign-in was cancelled.")),
            None,
        );
        login.kill();
        login.stop.notify_one();
        cancelled
    }

    /// Stop every login under way (the worker is stopping): a folder of
    /// theirs is removed, a Claude Code left would wait for nobody.
    pub fn cancel_all(&self) -> usize {
        let ids: Vec<String> = self
            .logins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .keys()
            .cloned()
            .collect();
        let mut stopped = 0;
        for id in ids {
            let login = self.get(&id);
            if self.cancel(&id) {
                stopped += 1;
            }
            if let Some(login) = login {
                let _ = std::fs::remove_dir_all(&login.folder);
            }
        }
        stopped
    }

    /// Start `claude auth login` for `plan` and answer once it printed its
    /// link. A login under way for the same account is cancelled first.
    pub async fn start(
        &self,
        accounts: Arc<Accounts>,
        plan: Plan,
        poke: Poke,
        gate: super::Gate,
    ) -> Result<ClaudeLogin, LoginError> {
        let before: Vec<String> = self
            .logins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|login| login.account == plan.account && !login.ended())
            .map(|login| login.id.clone())
            .collect();
        for id in before {
            self.cancel(&id);
        }
        let runner = self
            .runner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let claude = runner
            .claude
            .clone()
            .or_else(|| crate::binary::find_program("claude"))
            .ok_or_else(|| {
                login_error(
                    "failed",
                    "Claude Code is not installed on this computer (no `claude` on PATH or in ~/.local/bin).",
                )
            })?;
        sweep(&std::env::temp_dir(), now_ms());
        let id = uuid::Uuid::new_v4().simple().to_string();
        let folder = std::env::temp_dir().join(format!("{FOLDER_PREFIX}{id}"));
        let spawned = {
            let folder = folder.clone();
            let plan = plan.clone();
            let runner = runner.clone();
            tokio::task::spawn_blocking(move || spawn(&claude, &runner, &plan, &folder))
                .await
                .map_err(|error| {
                    login_error(
                        "failed",
                        format!("The worker failed to start Claude Code ({error})."),
                    )
                })?
        };
        // Claude Code runs where the plan said: a switch may go on.
        drop(gate);
        let spawned = match spawned {
            Ok(spawned) => spawned,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&folder);
                return Err(login_error(
                    "failed",
                    format!("Claude Code could not be started ({error})."),
                ));
            }
        };
        let started_at_ms = now_ms();
        let url = match wait_for_url(&spawned.output, spawned.exit.clone()).await {
            Ok(url) => url,
            Err(message) => {
                if let Some(pid) = spawned.pid.and_then(|pid| i32::try_from(pid).ok()) {
                    if spawned.exit.borrow().is_none() {
                        // SAFETY: as in Login::kill.
                        unsafe {
                            libc::kill(-pid, libc::SIGTERM);
                        }
                    }
                }
                drop(spawned.pty);
                let _ = std::fs::remove_dir_all(&folder);
                return Err(login_error("failed", message));
            }
        };
        let (status, _) = watch::channel(Status {
            state: LoginState::WaitingCode,
            error: None,
            ended_at_ms: None,
            signed_in_as: None,
            shapeless: 0,
        });
        let login = Arc::new(Login {
            id: id.clone(),
            account: plan.account.clone(),
            on_slot: plan.config_dir.is_none(),
            url,
            browser: plan.browser,
            email: plan.email.clone(),
            started_at_ms,
            expires_at_ms: started_at_ms + runner.ttl_ms.unwrap_or(LINK_TTL_MS),
            folder,
            pid: spawned.pid,
            status,
            exit: spawned.exit,
            pty: Mutex::new(Some(spawned.pty)),
            output: spawned.output,
            typed: Mutex::new(None),
            stop: Notify::new(),
            browser_link: Mutex::new(None),
        });
        tracing::info!(
            login = %id,
            account = %plan.account,
            on_slot = login.on_slot,
            browser = ?plan.browser,
            url = %redact_url(&login.url),
            "sign-in started"
        );
        if plan.browser == LoginBrowser::Host {
            // Claude Code hands the browser its link right after printing
            // the other one; the answer carries it when it came in time.
            let deadline = tokio::time::Instant::now() + BROWSER_NOTE_WAIT;
            while login.browser_url().is_none() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        self.logins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, login.clone());
        tokio::spawn(supervise(login.clone(), accounts, poke.clone()));
        poke();
        Ok(login.entry())
    }

    /// Type `code` into login `id` and wait for the outcome: the account
    /// it signed in (on the slot, the global one), or why not.
    pub async fn code(&self, id: &str, code: &str, poke: Poke) -> Result<String, LoginError> {
        let code = clean_code(code).ok_or_else(invalid_code)?;
        let login = self.get(id).ok_or_else(gone)?;
        let mut status = login.status.subscribe();
        let shapeless_before;
        {
            let current = status.borrow_and_update().clone();
            shapeless_before = current.shapeless;
            match current.state {
                LoginState::WaitingCode => {
                    // Checking before the code goes in, so whatever Claude
                    // Code answers finds it so.
                    *login.typed.lock().unwrap_or_else(|p| p.into_inner()) =
                        Some((login.output.len(), code.clone()));
                    login
                        .status
                        .send_modify(|status| status.state = LoginState::Checking);
                    let typed = {
                        let mut pty = login.pty.lock().unwrap_or_else(|p| p.into_inner());
                        match pty.as_mut() {
                            Some(pty) => {
                                let mut line = code.clone().into_bytes();
                                line.push(b'\r');
                                pty.writer
                                    .write_all(&line)
                                    .and_then(|()| pty.writer.flush())
                            }
                            None => Err(std::io::Error::other("closed")),
                        }
                    };
                    poke();
                    if typed.is_err() {
                        // Claude Code is gone; the supervisor says how.
                        let _ = status.borrow_and_update();
                    }
                }
                // Another device's code is being checked: same outcome.
                LoginState::Checking => {}
                _ => return finished(&current),
            }
        }
        let waited = tokio::time::timeout(CODE_WAIT, async {
            loop {
                {
                    let now = status.borrow_and_update();
                    if now.ended_at_ms.is_some() {
                        return finished(&now);
                    }
                    if now.state == LoginState::WaitingCode && now.shapeless > shapeless_before {
                        return Err(invalid_code());
                    }
                }
                if status.changed().await.is_err() {
                    return Err(gone());
                }
            }
        })
        .await;
        match waited {
            Ok(result) => result,
            Err(_) => {
                let error = login_error(
                    "failed",
                    "Claude Code didn't finish checking the code within a minute; get a new link.",
                );
                if login.end(LoginState::Failed, Some(error.clone()), None) {
                    login.kill();
                    login.stop.notify_one();
                    poke();
                }
                Err(finished(&login.status()).err().unwrap_or(error))
            }
        }
    }
}

/// The answer for a login that ended (or `gone`).
fn finished(status: &Status) -> Result<String, LoginError> {
    match (status.state, &status.signed_in_as, &status.error) {
        (LoginState::Done, Some(account), _) => Ok(account.clone()),
        (LoginState::Expired, _, _) => Err(expired()),
        (_, _, Some(error)) => Err(error.clone()),
        _ => Err(gone()),
    }
}

/// The login folders' names (in the temporary folder) start so.
const FOLDER_PREFIX: &str = "iii-tmux-login-";

/// Remove the login folders a worker that stopped without cleaning up
/// left in `temp`: older than any login can live.
fn sweep(temp: &Path, now: u64) {
    let Ok(entries) = std::fs::read_dir(temp) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(FOLDER_PREFIX)
        {
            continue;
        }
        let old = entry
            .metadata()
            .ok()
            .filter(|meta| meta.is_dir())
            .and_then(|meta| meta.modified().ok())
            .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
            .is_some_and(|at| now.saturating_sub(at.as_millis() as u64) > LINK_TTL_MS + LINGER_MS);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

struct Spawned {
    pid: Option<u32>,
    pty: Pty,
    output: Arc<Output>,
    exit: watch::Receiver<Option<Option<i32>>>,
}

/// The `PATH` Claude Code gets: the shims' folder first when no browser
/// may open (a second guard after `BROWSER`, WD2).
fn path_with(first: Option<&Path>) -> std::ffi::OsString {
    let rest = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = first.map(Path::to_path_buf).into_iter().collect();
    dirs.extend(std::env::split_paths(&rest));
    std::env::join_paths(dirs).unwrap_or(rest)
}

/// What opens a link on this computer for `browser: host`: the worker's
/// own `BROWSER` (one program), else `open` on macOS, `xdg-open` on Linux.
fn host_opener() -> Option<PathBuf> {
    if let Some(browser) = std::env::var_os("BROWSER").filter(|b| !b.is_empty()) {
        let browser = PathBuf::from(browser);
        let text = browser.to_string_lossy();
        if !text.contains(char::is_whitespace) && !text.contains(':') {
            return if browser.is_absolute() {
                Some(browser)
            } else {
                crate::binary::find_program(&text)
            };
        }
    }
    if cfg!(target_os = "macos") {
        Some(PathBuf::from("/usr/bin/open"))
    } else {
        crate::binary::find_program("xdg-open")
    }
}

fn write_script(path: &Path, text: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(path)?;
    file.write_all(text.as_bytes())
}

/// Make the login's folder (0700: the shims and the link they note), then
/// start Claude Code in a pty.
fn spawn(claude: &Path, runner: &Runner, plan: &Plan, folder: &Path) -> anyhow::Result<Spawned> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(folder)?;
    let note = folder.join("browser-link");
    let opener = match plan.browser {
        LoginBrowser::Host => runner.opener.clone().or_else(host_opener),
        LoginBrowser::None => None,
    };
    let shim = folder.join("browser");
    write_script(&shim, &browser_shim(&note, opener.as_deref()))?;
    let guard = match plan.browser {
        LoginBrowser::None => {
            let bin = folder.join("bin");
            std::fs::DirBuilder::new().mode(0o700).create(&bin)?;
            for name in ["open", "xdg-open"] {
                write_script(&bin.join(name), &browser_shim(&note, None))?;
            }
            Some(bin)
        }
        LoginBrowser::Host => None,
    };

    let pair = native_pty_system().openpty(PtySize {
        rows: ROWS,
        cols: COLS,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut command = CommandBuilder::new(claude);
    command.args(["auth", "login"]);
    if let Some(email) = plan.email.as_deref().filter(|email| !email.is_empty()) {
        command.args(["--email", email]);
    }
    command.cwd(&plan.home);
    for key in STRIPPED_ENV {
        command.env_remove(key);
    }
    command.env("HOME", &plan.home);
    command.env("TERM", "xterm-256color");
    command.env("BROWSER", &shim);
    command.env("PATH", path_with(guard.as_deref()));
    match &plan.config_dir {
        Some(dir) => command.env("CLAUDE_CONFIG_DIR", dir),
        None => command.env_remove("CLAUDE_CONFIG_DIR"),
    }
    for (key, value) in &runner.env {
        command.env(key, value);
    }
    let mut child = pair.slave.spawn_command(command)?;
    drop(pair.slave);
    let pid = child.process_id();
    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let output = Arc::new(Output::default());
    {
        let output = output.clone();
        std::thread::Builder::new()
            .name("tmux-login-read".into())
            .spawn(move || {
                let mut chunk = [0u8; 8192];
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => output.push(&chunk[..n]),
                    }
                }
            })?;
    }
    let (exit_tx, exit) = watch::channel(None);
    std::thread::Builder::new()
        .name("tmux-login-wait".into())
        .spawn(move || {
            let status = child.wait().ok().map(|status| status.exit_code() as i32);
            let _ = exit_tx.send(Some(status));
        })?;
    Ok(Spawned {
        pid,
        pty: Pty {
            _master: pair.master,
            writer,
        },
        output,
        exit,
    })
}

/// The link, once Claude Code printed it; why not otherwise.
async fn wait_for_url(
    output: &Output,
    mut exit: watch::Receiver<Option<Option<i32>>>,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + URL_WAIT;
    loop {
        let text = output.text_from(0);
        if let Some(url) = find_url(&text) {
            return Ok(url);
        }
        let ended = *exit.borrow_and_update();
        if let Some(status) = ended {
            // What it printed last may still be on its way.
            tokio::time::sleep(Duration::from_millis(100)).await;
            let text = output.text_from(0);
            return Err(early_end(&text, status));
        }
        tokio::select! {
            _ = output.changed.notified() => {}
            _ = exit.changed() => {}
            _ = tokio::time::sleep_until(deadline) => {
                return Err(format!(
                    "Claude Code showed no sign-in link within {} seconds.",
                    URL_WAIT.as_secs()
                ));
            }
        }
    }
}

/// Watch a login to its end: Claude Code exits (signed in or not), its 10
/// minutes run out, or someone cancels it.
async fn supervise(login: Arc<Login>, accounts: Arc<Accounts>, poke: Poke) {
    let mut exit = login.exit.clone();
    let wait = Duration::from_millis(login.expires_at_ms.saturating_sub(now_ms()));
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        if login.ended() {
            break;
        }
        let ended = *exit.borrow_and_update();
        if let Some(status) = ended {
            // Its last words may still be in the pty.
            tokio::time::sleep(Duration::from_millis(150)).await;
            settle(&login, &accounts, status).await;
            break;
        }
        note_shapeless(&login, &poke);
        tokio::select! {
            _ = exit.changed() => {}
            _ = login.output.changed.notified() => {}
            _ = login.stop.notified() => {}
            _ = tokio::time::sleep_until(deadline) => {
                if login.end(LoginState::Expired, Some(expired()), None) {
                    tracing::info!(login = %login.id, "sign-in link expired");
                    login.kill();
                }
                break;
            }
        }
    }
    login.kill();
    let _ = std::fs::remove_dir_all(&login.folder);
    poke();
}

/// Claude Code refused a line for its shape and waits again (WD5): the
/// login goes back to waiting for a code.
fn note_shapeless(login: &Login, poke: &Poke) {
    let typed_at = login
        .typed
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|(at, _)| *at);
    let Some(at) = typed_at else {
        return;
    };
    if login.status.borrow().state != LoginState::Checking {
        return;
    }
    if strip_escapes(&login.output.text_from(at)).contains(SHAPELESS) {
        *login.typed.lock().unwrap_or_else(|p| p.into_inner()) = None;
        login.status.send_modify(|status| {
            status.state = LoginState::WaitingCode;
            status.shapeless += 1;
        });
        poke();
    }
}

/// Claude Code ended with `status`: signed in or not.
async fn settle(login: &Login, accounts: &Arc<Accounts>, status: Option<i32>) {
    let typed = login
        .typed
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let (at, code) = match &typed {
        Some((at, code)) => (*at, Some(code.as_str())),
        None => (0, None),
    };
    let text = login.output.text_from(at);
    match outcome(status == Some(0), &text, code) {
        Outcome::Done => {
            let account = signed_in_as(login, accounts).await;
            tracing::info!(login = %login.id, account = %account, "signed in");
            login.end(LoginState::Done, None, Some(account));
        }
        Outcome::Rejected(message) => {
            tracing::info!(login = %login.id, "Anthropic did not accept the code");
            login.end(
                LoginState::Failed,
                Some(login_error("rejected", message)),
                None,
            );
        }
        Outcome::Failed(message) => {
            tracing::info!(login = %login.id, status = ?status, "sign-in failed");
            login.end(
                LoginState::Failed,
                Some(login_error("failed", message)),
                None,
            );
        }
    }
}

/// The account a finished sign-in landed on: its own on its folder; on
/// the slot, the global account once the worker took in a sign-in to
/// another one (G6), which happens here rather than at the next tick.
async fn signed_in_as(login: &Login, accounts: &Arc<Accounts>) -> String {
    accounts.reread_credentials(&login.account);
    if !login.on_slot {
        accounts
            .load_accounts_now(vec![login.account.clone()])
            .await;
        return login.account.clone();
    }
    {
        let _gate = accounts.gate().await;
        accounts.watch_slot().await;
    }
    let global = accounts.registry().global;
    accounts.reread_credentials(&global);
    accounts.load_now().await;
    global
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = "Opening browser to sign in\u{2026}\r\nIf the browser didn't open, visit: \u{1b}]8;;https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=user%3Aprofile+user%3Ainference&code_challenge=abc&code_challenge_method=S256&state=xyz\u{7}\u{1b}[94mhttps://claude.com/cai/oauth/authorize?code=true&client_id=9d1c&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=user%3Aprofile+user%3Ainference&code_challenge=abc&code_challenge_method=S256&state=xyz\u{1b}[39m\u{1b}]8;;\u{7}\r\nPaste code here if prompted > ";
    const URL: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=user%3Aprofile+user%3Ainference&code_challenge=abc&code_challenge_method=S256&state=xyz";

    #[test]
    fn codes_lose_their_whitespace_and_need_two_halves() {
        assert_eq!(clean_code(" abc#def \n"), Some("abc#def".into()));
        assert_eq!(clean_code("ab\tc\n#d e f"), Some("abc#def".into()));
        assert_eq!(clean_code("a#b#c"), Some("a#b#c".into()));
        for bad in [
            "",
            "   ",
            "abcdef",
            "#def",
            "abc#",
            "#",
            "https://claude.ai/x",
            "abc#d\u{e9}f",
            "abc#\u{1b}[A",
        ] {
            assert_eq!(clean_code(bad), None, "{bad:?}");
        }
        assert_eq!(clean_code(&format!("{}#x", "a".repeat(CODE_MAX))), None);
    }

    #[test]
    fn the_link_is_read_from_claude_code_s_osc_8_hyperlink() {
        assert_eq!(find_url(REAL).as_deref(), Some(URL));
        // Arriving: the hyperlink isn't complete yet, nor the plain text.
        let half = &REAL[..REAL.find("state=xyz").unwrap() + 7];
        assert_eq!(find_url(half), None);
    }

    #[test]
    fn plain_links_count_once_a_line_end_follows() {
        let plain = format!("Sign in at: {URL}\r\nPaste code here > ");
        assert_eq!(find_url(&plain).as_deref(), Some(URL));
        assert_eq!(find_url(&format!("Sign in at: {URL}")), None);
        let coloured = format!("visit: \u{1b}[94m{URL}\u{1b}[39m\r\n");
        assert_eq!(find_url(&coloured).as_deref(), Some(URL));
        // Some other link isn't the sign-in page.
        assert_eq!(find_url("see https://claude.com/docs \r\n"), None);
        // OSC 8 ended by ESC \.
        let st = format!("\u{1b}]8;id=1;{URL}\u{1b}\\{URL}\u{1b}]8;;\u{1b}\\");
        assert_eq!(find_url(&st).as_deref(), Some(URL));
    }

    #[test]
    fn escapes_go_and_text_stays() {
        assert_eq!(
            strip_escapes("a\u{1b}[94mb\u{1b}[39m\u{1b}]8;;x\u{7}c\u{1b}]2;t\u{1b}\\d\u{1b}[?25h"),
            "abcd"
        );
    }

    #[test]
    fn logged_links_keep_no_state_challenge_or_email() {
        let url = format!("{URL}&login_hint=you%40home.dev");
        let redacted = redact_url(&url);
        assert!(
            !redacted.contains("xyz")
                && !redacted.contains("abc")
                && !redacted.contains("home.dev")
        );
        assert!(redacted.contains("code_challenge=…") && redacted.contains("client_id=9d1c"));
        assert_eq!(redact_url("https://x/y"), "https://x/y");
    }

    #[test]
    fn outcomes_follow_what_claude_code_said() {
        assert_eq!(
            outcome(true, "Login successful.\r\n", Some("a#b")),
            Outcome::Done
        );
        let refused = "a#b\r\nLogin failed: That code wasn't accepted. It may be incomplete, expired, already used, or from an earlier sign-in attempt. Sign in again to get a new code.\r\n\u{1b}[?25h";
        match outcome(false, refused, Some("a#b")) {
            Outcome::Rejected(line) => assert!(line.starts_with("That code wasn't accepted.")),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            outcome(
                false,
                "Login failed: Authentication failed: Invalid authorization code\r\n",
                None
            ),
            Outcome::Rejected(_)
        ));
        assert_eq!(
            outcome(
                false,
                "Login failed: Token exchange failed (500): boom\r\n",
                None
            ),
            Outcome::Failed("Token exchange failed (500): boom".into())
        );
        assert_eq!(
            outcome(false, "secret#code\r\n", Some("secret#code")),
            Outcome::Failed("Claude Code ended without signing in.".into())
        );
        // The code never comes back in a message, even if Claude Code said it.
        assert_eq!(
            outcome(
                false,
                "Login failed: bad secret#code here\r\n",
                Some("secret#code")
            ),
            Outcome::Failed("bad … here".into())
        );
    }

    #[test]
    fn an_early_end_says_why() {
        assert_eq!(
            early_end("Error: --console and --claudeai cannot be used together.\r\n", Some(1)),
            "Claude Code ended before it showed a sign-in link: Error: --console and --claudeai cannot be used together."
        );
        assert_eq!(
            early_end("Login failed: offline\r\n", Some(1)),
            "Claude Code could not start the sign-in: offline"
        );
        assert_eq!(
            early_end("", Some(127)),
            "Claude Code ended before it showed a sign-in link (exit status 127)."
        );
    }

    #[test]
    fn the_browser_shim_notes_the_link_and_opens_only_on_the_host() {
        let none = browser_shim(Path::new("/t/l 1/browser-link"), None);
        assert!(none.starts_with("#!/bin/sh\n"));
        assert!(none.contains("> '/t/l 1/browser-link'.part && mv -f '/t/l 1/browser-link'.part '/t/l 1/browser-link'"));
        assert!(none.ends_with("exit 0\n"));
        let host = browser_shim(Path::new("/t/n"), Some(Path::new("/usr/bin/open")));
        assert!(host.ends_with("exec /usr/bin/open \"$1\"\n"));
    }

    #[test]
    fn the_shim_runs_and_opens_nothing() {
        let dir =
            std::env::temp_dir().join(format!("tmux-login-shim-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let note = dir.join("browser-link");
        let shim = dir.join("browser");
        write_script(&shim, &browser_shim(&note, None)).unwrap();
        let ran = std::process::Command::new(&shim)
            .arg("http://localhost:5555/callback?code=x&state=y")
            .status()
            .unwrap();
        assert!(ran.success());
        assert_eq!(
            std::fs::read_to_string(&note).unwrap(),
            "http://localhost:5555/callback?code=x&state=y\n"
        );
        // A trap after it on the way: never reached.
        let opener = dir.join("opener");
        write_script(
            &opener,
            &format!(
                "#!/bin/sh\necho \"$1\" > {}\n",
                shell_quote(&dir.join("opened").to_string_lossy())
            ),
        )
        .unwrap();
        let host = dir.join("host");
        write_script(&host, &browser_shim(&note, Some(&opener))).unwrap();
        assert!(std::process::Command::new(&host)
            .arg("http://localhost:1/x")
            .status()
            .unwrap()
            .success());
        assert_eq!(
            std::fs::read_to_string(dir.join("opened")).unwrap(),
            "http://localhost:1/x\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_login_folders_are_swept() {
        let temp = std::env::temp_dir().join(format!(
            "tmux-login-sweep-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let fresh = temp.join(format!("{FOLDER_PREFIX}fresh"));
        let other = temp.join("something-else");
        std::fs::create_dir_all(&fresh).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        sweep(&temp, now_ms());
        assert!(fresh.is_dir() && other.is_dir());
        sweep(&temp, now_ms() + LINK_TTL_MS + LINGER_MS + 60_000);
        assert!(!fresh.exists() && other.is_dir());
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn errors_go_out_as_one_json_object() {
        let wire: serde_json::Value = serde_json::from_str(&to_wire(&invalid_code())).unwrap();
        assert_eq!(wire["code"], "invalid_code");
        assert!(wire["message"].as_str().unwrap().contains('#'));
    }
}
