//! Moving a Claude Code pane to another account (agent-accounts D6–D9,
//! §3.7): Claude Code stops, the worker makes sure the other account's
//! folder sees the conversation, and Claude Code starts again there with
//! `--resume <session>`. Stopped first, Claude Code has written the end of
//! its conversation before it is copied.
//!
//! - Right before it is stopped, the pane is looked at again: the same
//!   program, Claude Code's own status still exactly `idle` (D7). Otherwise
//!   nothing changes and the move waits for the turn (`after_turn`).
//! - A pane whose program is Claude Code (the worker started it, or
//!   `sh -c` did) is kept with remain-on-exit while Claude Code ends on
//!   SIGTERM, then respawned with the new command; its remain-on-exit is put
//!   back as it was (unset stays unset).
//! - Claude Code started by hand in an interactive shell ends on SIGTERM;
//!   once the shell has the terminal back, it gets the command typed (`C-e`
//!   `C-u` to clear the line, the command, Enter). If the shell doesn't
//!   take the terminal back, nothing is typed. It is always the explicit
//!   form (`env -u CLAUDE_CONFIG_DIR` for the slot, `env
//!   CLAUDE_CONFIG_DIR=…` for a folder): the shell kept the environment it
//!   started with.
//! - A switch of the global account (`switch`) stops pinned panes before
//!   it swaps the sign-ins and starts them after ([`stop`],
//!   [`Stopped::start`]).
//! - When the conversation can't be copied (the other account has a longer
//!   copy of it, say), Claude Code starts again where it was, resuming the
//!   same conversation, and the move fails.
//!
//! Claude Code's own shutdown can take more than D6's five seconds (its
//! failsafe waits at least five, plus SessionEnd hooks), so the worker
//! waits up to ten; if it still runs, the move fails and nothing else is
//! changed. SIGTERM was sent, though: a respawned pane keeps remain-on-exit
//! until Claude Code does end, then Claude Code starts again on its own
//! account, resuming its conversation, and remain-on-exit is put back. A
//! shell simply gets the terminal back.
//!
//! A move happens only between turns (D7); otherwise the pane gets
//! `@iii_claude_pending` and the worker moves it when it is idle.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::launch::{MARK_ACCOUNT, MARK_PENDING, MARK_SKIP};
use super::panes::{MoveKind, PaneState, NOT_RUNNING, NO_SESSION};
use super::registry::{self, Registry};
use super::slot::Place;
use super::{Accounts, GLOBAL};
use crate::tmux_cli::Tmux;

/// How long Claude Code gets to end on SIGTERM.
pub const STOP_WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(100);

pub const DID_NOT_STOP: &str = "Claude Code did not stop within ten seconds; once it does, it starts again on its account, resuming its conversation.";
pub const DID_NOT_STOP_SHELL: &str =
    "Claude Code did not stop within ten seconds; its shell gets the terminal back once it does.";
/// How long a pane whose Claude Code did not stop in time is watched for
/// its end, keeping remain-on-exit on.
const LATE_STOP_WAIT: Duration = Duration::from_secs(15 * 60);
pub const NO_TRANSCRIPT: &str = "Claude Code's conversation file was not found.";
pub const BUSY: &str = "This pane is being moved already.";
/// Claude Code is no longer idle when the worker looks again: the move
/// waits for its turn instead.
pub const NOT_IDLE: &str = "Claude Code is in a turn now; it moves once the turn ends.";
pub const PANE_CHANGED: &str = "The pane runs another program now; it was left as it was.";
/// Claude Code's session record can't be matched to its process (its start
/// time is missing or another one's): nothing is stopped.
pub const NOT_SAME_PROCESS: &str =
    "Claude Code's session record can't be matched to its process; it was left as it was.";
pub const SHELL_NOT_BACK: &str =
    "Claude Code stopped, but its shell did not take the terminal back, so nothing was typed.";
pub const NEWER_COPY: &str =
    "The other account has a longer copy of this conversation; it was not overwritten.";
/// How long a shell gets to take the terminal back after Claude Code ends.
const SHELL_WAIT: Duration = Duration::from_secs(3);

/// What `tmux::panes::account` and the switches do to a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Only the marks change: Claude Code already runs where it should, or
    /// doesn't run.
    Marks {
        mark: String,
        to: String,
        state: &'static str,
    },
    /// Move it now (it is idle) to `place`.
    Now {
        mark: String,
        to: String,
        place: Place,
    },
    /// Move it to `place` once its turn ends.
    AfterTurn {
        mark: String,
        to: String,
        place: Place,
    },
}

/// Where a pane marked `mark` (`global` or an id) runs, with `registry`
/// naming the global account: the slot for `global` and the global
/// account's id, the account's folder otherwise (G5).
pub fn place_for(mark: &str, registry: &Registry) -> Place {
    if mark == GLOBAL || mark == registry.global {
        Place::Slot
    } else {
        Place::Folder(mark.to_string())
    }
}

/// The account a pane marked `mark` uses.
fn account_for(mark: &str, registry: &Registry) -> String {
    if mark == GLOBAL {
        registry.global.clone()
    } else {
        mark.to_string()
    }
}

/// The plan for `want` (`global` or an id) on `pane`, `registry` naming
/// the accounts and the global one (G12): `global` runs on the slot and
/// follows; the global account's id pins the pane there (on the slot,
/// without a restart); another id runs on that account's folder.
pub fn plan(pane: &PaneState, registry: &Registry, want: &str) -> Result<Plan, String> {
    let want = want.trim();
    if want != GLOBAL && !registry.has(want) {
        return Err(super::manage::no_account(want));
    }
    if pane.marks_shared {
        return Err(super::panes::SHARED_MARKS.to_string());
    }
    let mark = want.to_string();
    let to = account_for(want, registry);
    let place = place_for(want, registry);
    let settled = |pending_cancelled: bool| {
        let state = if pending_cancelled {
            "cancelled"
        } else if want == GLOBAL {
            "following"
        } else {
            "pinned"
        };
        Plan::Marks {
            mark: mark.clone(),
            to: to.clone(),
            state,
        }
    };
    if !pane.running() {
        return Ok(settled(false));
    }
    if pane.place.as_ref() == Some(&place) {
        return Ok(settled(pane.marks.pending.is_some()));
    }
    if !pane.info.movable || pane.kind.is_none() {
        return Err(pane.why_not_movable());
    }
    Ok(if pane.idle() {
        Plan::Now { mark, to, place }
    } else {
        Plan::AfterTurn { mark, to, place }
    })
}

/// `projects/` folder name Claude Code gives `cwd`.
fn project_slug(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The transcript of session `id` under config folder `dir`:
/// `projects/<folder>/<id>.jsonl`, the cwd's folder first.
pub fn find_transcript(dir: &Path, id: &str, cwd: Option<&str>) -> Option<PathBuf> {
    let projects = dir.join("projects");
    let file = format!("{id}.jsonl");
    if let Some(cwd) = cwd {
        let direct = projects.join(project_slug(cwd)).join(&file);
        if direct.is_file() {
            return Some(direct);
        }
    }
    std::fs::read_dir(&projects)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(&file))
        .find(|path| path.is_file())
}

fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Whether `from` may replace `to`: `to` is missing, or `from` is longer
/// (a transcript only grows), or as long and not older.
fn may_replace(from: &Path, to: &Path) -> Result<bool, String> {
    let source = std::fs::metadata(from)
        .map_err(|error| format!("Could not read {} ({error}).", from.display()))?;
    let Ok(target) = std::fs::symlink_metadata(to) else {
        return Ok(true);
    };
    if !target.is_file() {
        return Ok(false);
    }
    let newer = match (source.modified(), target.modified()) {
        (Ok(source), Ok(target)) => source >= target,
        _ => false,
    };
    Ok(source.len() > target.len() || (source.len() == target.len() && newer))
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    let folder = to
        .parent()
        .ok_or_else(|| "No folder to copy into.".to_string())?;
    let temp = folder.join(format!(
        ".{}.{}.tmp",
        to.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file"),
        uuid::Uuid::new_v4().simple()
    ));
    let copied = std::fs::copy(from, &temp).and_then(|_| std::fs::rename(&temp, to));
    if let Err(error) = copied {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Could not copy {} ({error}).", from.display()));
    }
    Ok(())
}

/// Copy the files under `from` into `to`, keeping its layout; links are
/// not followed.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    registry::make_private_dir(to)?;
    for entry in std::fs::read_dir(from)
        .map_err(|error| format!("Could not read {} ({error}).", from.display()))?
        .flatten()
    {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() && may_replace(&entry.path(), &target)? {
            // A file the other account has a longer copy of stays.
            copy_file(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Make account folder `to_dir` see session `id`'s conversation, which
/// lives under `from_dir`: unless it is already the same file (both share
/// `projects/`), copy `<id>.jsonl` and its `<id>/` folder into the same
/// project folder there. A copy there that is longer than this one (the
/// conversation went on in that account) is never overwritten: that is an
/// error. Returns whether it copied.
pub fn share_transcript(
    from_dir: &Path,
    to_dir: &Path,
    id: &str,
    cwd: Option<&str>,
) -> Result<bool, String> {
    let source = find_transcript(from_dir, id, cwd).ok_or_else(|| NO_TRANSCRIPT.to_string())?;
    let project = source
        .parent()
        .and_then(Path::file_name)
        .ok_or_else(|| NO_TRANSCRIPT.to_string())?;
    let folder = to_dir.join("projects").join(project);
    let target = folder.join(format!("{id}.jsonl"));
    if same_file(&source, &target) {
        return Ok(false);
    }
    if !may_replace(&source, &target)? {
        return Err(NEWER_COPY.to_string());
    }
    registry::make_private_dir(&folder)?;
    copy_file(&source, &target)?;
    let sibling = source.with_extension("");
    if sibling.is_dir() {
        copy_tree(&sibling, &folder.join(id))?;
    }
    Ok(true)
}

/// One move at a time per pane.
#[derive(Debug, Default)]
pub struct Moving(std::sync::Arc<std::sync::Mutex<HashSet<String>>>);

/// A pane being moved; another move of it is refused ([`BUSY`]) until this
/// is dropped, which a pane whose Claude Code stops late does only once it
/// has ended ([`after_late_stop`]).
pub struct MoveGuard {
    moving: std::sync::Arc<std::sync::Mutex<HashSet<String>>>,
    pane: String,
}

impl Moving {
    /// Whether `pane` is being moved now (a late stop included).
    pub fn is_busy(&self, pane: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(pane)
    }

    /// Whether any pane is being moved now (a late stop included).
    pub fn any(&self) -> bool {
        !self.0.lock().unwrap_or_else(|p| p.into_inner()).is_empty()
    }

    pub fn begin(&self, pane: &str) -> Option<MoveGuard> {
        let mut set = self.0.lock().unwrap_or_else(|p| p.into_inner());
        set.insert(pane.to_string()).then(|| MoveGuard {
            moving: self.0.clone(),
            pane: pane.to_string(),
        })
    }
}

impl Drop for MoveGuard {
    fn drop(&mut self) {
        self.moving
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.pane);
    }
}

/// The marks a pane gets: its account mark, the skip mark set or cleared,
/// and `pending` set or cleared. As tmux option commands.
pub fn mark_args<'a>(
    scope: &'a str,
    pane: &'a str,
    mark: &'a str,
    skip: bool,
    pending: Option<&'a str>,
) -> Vec<Vec<&'a str>> {
    let mut commands = vec![vec!["set-option", scope, "-t", pane, MARK_ACCOUNT, mark]];
    commands.push(if skip {
        vec!["set-option", scope, "-t", pane, MARK_SKIP, "1"]
    } else {
        vec!["set-option", scope, "-u", "-t", pane, MARK_SKIP]
    });
    commands.push(match pending {
        Some(to) => vec!["set-option", scope, "-t", pane, MARK_PENDING, to],
        None => vec!["set-option", scope, "-u", "-t", pane, MARK_PENDING],
    });
    commands
}

/// `commands` as one tmux invocation, `;` between them.
fn chain<'a>(commands: impl IntoIterator<Item = Vec<&'a str>>) -> Vec<&'a str> {
    let mut args = Vec::new();
    for command in commands {
        if !args.is_empty() {
            args.push(";");
        }
        args.extend(command);
    }
    args
}

/// Set `pane`'s marks without touching its process.
pub async fn set_marks(
    tmux: &Tmux,
    pane: &str,
    mark: &str,
    skip: bool,
    pending: Option<&str>,
) -> Result<(), String> {
    let scope = tmux.pane_scope();
    tmux.run(chain(mark_args(scope, pane, mark, skip, pending)))
        .await
        .map(drop)
        .map_err(|error| error.sentence())
}

fn signal(pid: u32, signal: libc::c_int) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: kill has no memory preconditions; a pid that is gone is ESRCH.
    unsafe { libc::kill(pid, signal) == 0 }
}

fn alive(pid: u32) -> bool {
    signal(pid, 0)
}

async fn pane_field(tmux: &Tmux, pane: &str, field: &str) -> Result<String, String> {
    Ok(tmux.pane_fields(pane, &[field]).await?.remove(0))
}

/// Wait for `done`, at most `limit`.
async fn wait_for<F, Fut>(limit: Duration, mut done: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + limit;
    loop {
        if done().await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(POLL).await;
    }
}

/// A tmux option set on `target` itself at `scope` (`-p` for a pane, `-w`
/// for its window before tmux 3.0): `None` when it is not set there (it
/// inherits), never the inherited value.
async fn local_option(
    tmux: &Tmux,
    scope: &str,
    target: &str,
    name: &str,
) -> Result<Option<String>, String> {
    // Without `-A`, only what is set at that level: nothing when it inherits.
    let shown = tmux
        .run(["show-options", scope, "-v", "-t", target, name])
        .await
        .map_err(|error| error.sentence())?;
    Ok(Some(shown.trim().to_string()).filter(|value| !value.is_empty()))
}

/// The tmux command that puts `name` back as `before` said.
fn restore_option<'a>(
    scope: &'a str,
    target: &'a str,
    name: &'a str,
    before: Option<&'a str>,
) -> Vec<&'a str> {
    match before {
        Some(value) => vec!["set-option", scope, "-t", target, name, value],
        None => vec!["set-option", scope, "-u", "-t", target, name],
    }
}

/// The keys typed into a shell to run `command`: to the end of the line,
/// clear it (`C-u` clears to the start in bash and fish, the whole line in
/// zsh), the command as text, Enter.
pub fn shell_keys<'a>(pane: &'a str, command: &'a str) -> Vec<Vec<&'a str>> {
    vec![
        vec!["send-keys", "-t", pane, "C-e"],
        vec!["send-keys", "-t", pane, "C-u"],
        vec!["send-keys", "-t", pane, "-l", command],
        vec!["send-keys", "-t", pane, "Enter"],
    ]
}

/// D7, looked at again right before the signal: the pane runs the same
/// program, Claude Code is still the same process under it (the pid's
/// parent chain reaches the pane's program, and its session record's start
/// time is the process's own, both known), in the same conversation, and
/// says exactly `idle`. Blocking reads go off the async threads.
async fn check_still_idle(
    tmux: &Tmux,
    pane: &PaneState,
    claude_pid: u32,
    session: &str,
) -> Result<(), &'static str> {
    let fields = tmux
        .pane_fields(&pane.pane_id, &["#{pane_pid}", "#{pane_dead}"])
        .await
        .map_err(|_| PANE_CHANGED)?;
    if fields.first().map(String::as_str) != Some(pane.pane_pid.to_string().as_str())
        || fields.get(1).map(String::as_str) == Some("1")
    {
        return Err(PANE_CHANGED);
    }
    let pane_pid = pane.pane_pid;
    let (under_pane, record) = tokio::task::spawn_blocking(move || {
        (
            crate::procs::descends_from(claude_pid, pane_pid),
            crate::activity::claude_record_now(claude_pid, true),
        )
    })
    .await
    .map_err(|_| NOT_SAME_PROCESS)?;
    if !under_pane {
        return Err(PANE_CHANGED);
    }
    let record = record.ok_or(NOT_SAME_PROCESS)?;
    if record.session_id.as_deref() != Some(session) {
        return Err(NOT_IDLE);
    }
    match record.status.as_deref() {
        Some(status) if super::panes::is_idle_status(status) => Ok(()),
        _ => Err(NOT_IDLE),
    }
}

/// Make `to_dir` see the conversation, off the async threads.
async fn share(
    record: &crate::activity::ClaudeRecord,
    to_dir: PathBuf,
    session: &str,
) -> Result<bool, String> {
    let (from_dir, id, cwd) = (
        record.config_dir.clone(),
        session.to_string(),
        record.cwd.clone(),
    );
    tokio::task::spawn_blocking(move || share_transcript(&from_dir, &to_dir, &id, cwd.as_deref()))
        .await
        .map_err(|error| format!("The worker failed to copy the conversation ({error})."))?
}

/// A Claude Code the worker stopped (D6), whose pane waits for the
/// command that starts it again: [`Stopped::start`] with a new one, or
/// [`Stopped::start_where_it_was`]. No other move of the pane happens until
/// then.
pub struct Stopped {
    _guard: MoveGuard,
    pub pane_id: String,
    kind: MoveKind,
    /// The tmux command that puts remain-on-exit back (a respawned pane).
    restore: Vec<String>,
    /// The pane's folder, as a tmux format literal.
    cwd: String,
    pub session: String,
    pub record: crate::activity::ClaudeRecord,
    pub skip: bool,
    /// Where it ran.
    pub from: Place,
    /// What starts it there again.
    stay: String,
}

/// Stop the Claude Code of `pane` (as a fresh collection saw it) for a
/// move: looked at again right before the signal ([`NOT_IDLE`] when it is
/// no longer idle, nothing changed), SIGTERM, then the pane dead (kept with
/// remain-on-exit) or its shell back in front.
pub async fn stop(tmux: &Tmux, accounts: &Accounts, pane: &PaneState) -> Result<Stopped, String> {
    let guard = accounts.moving.begin(&pane.pane_id).ok_or(BUSY)?;
    let claude_pid = pane.claude_pid.ok_or(NOT_RUNNING)?;
    let kind = pane.kind.ok_or_else(|| pane.why_not_movable())?;
    let record = pane.record.clone().ok_or(NO_SESSION)?;
    let session = record.session_id.clone().ok_or(NO_SESSION)?;
    let from = pane
        .place
        .clone()
        .ok_or("Claude Code runs on a folder that is no account here.")?;
    let skip = pane.info.skip_permissions;
    // Back where it was, should anything after the stop fail.
    let stay = accounts.restart_command(&from, skip, Some(&session))?;
    let cwd = record
        .cwd
        .clone()
        .filter(|cwd| Path::new(cwd).is_dir())
        .unwrap_or_else(|| pane.cwd.clone());
    let cwd = crate::tmux_cli::format_literal(&cwd);
    let scope = tmux.pane_scope();
    let id = pane.pane_id.as_str();
    let mut restore = Vec::new();
    match kind {
        MoveKind::Respawn => {
            let before = local_option(tmux, scope, id, "remain-on-exit").await?;
            restore = restore_option(scope, id, "remain-on-exit", before.as_deref())
                .into_iter()
                .map(str::to_string)
                .collect();
            tmux.run(["set-option", scope, "-t", id, "remain-on-exit", "on"])
                .await
                .map_err(|error| error.sentence())?;
            // D7, looked at again right before the signal: a turn may have
            // begun since the collection.
            if let Err(error) = check_still_idle(tmux, pane, claude_pid, &session).await {
                let _ = tmux.run(&restore).await;
                return Err(error.to_string());
            }
            tracing::info!(pane = %id, "stopping Claude Code to move it");
            signal(claude_pid, libc::SIGTERM);
            let dead = wait_for(STOP_WAIT, || async {
                pane_field(tmux, id, "#{pane_dead}").await.as_deref() == Ok("1")
            })
            .await;
            if !dead {
                // SIGTERM was sent: the pane must not close when Claude
                // Code ends at last. remain-on-exit stays on until then.
                after_late_stop(
                    guard,
                    tmux.clone(),
                    id.to_string(),
                    pane.pane_pid,
                    cwd,
                    stay,
                    restore,
                );
                return Err(DID_NOT_STOP.to_string());
            }
        }
        MoveKind::Shell => {
            check_still_idle(tmux, pane, claude_pid, &session).await?;
            tracing::info!(pane = %id, "stopping Claude Code started in a shell to move it");
            signal(claude_pid, libc::SIGTERM);
            let gone = wait_for(STOP_WAIT, || async { !alive(claude_pid) }).await;
            if !gone {
                // No other move of this pane (and no second SIGTERM) until
                // this Claude Code has ended.
                tokio::spawn(async move {
                    let _guard = guard;
                    let _ = wait_for(LATE_STOP_WAIT, || async { !alive(claude_pid) }).await;
                });
                return Err(DID_NOT_STOP_SHELL.to_string());
            }
            // The shell takes the terminal back as soon as its job ends;
            // typing before it has would go to something else.
            let back = wait_for(SHELL_WAIT, || async {
                pane_field(tmux, id, "#{pane_current_command}")
                    .await
                    .is_ok_and(|now| super::panes::is_shell_name(&now))
            })
            .await;
            if !back {
                return Err(SHELL_NOT_BACK.to_string());
            }
        }
    }
    Ok(Stopped {
        _guard: guard,
        pane_id: pane.pane_id.clone(),
        kind,
        restore,
        cwd,
        session,
        record,
        skip,
        from,
        stay,
    })
}

impl Stopped {
    /// Start Claude Code again with `command`, the pane marked `mark`
    /// first (with no pending move) when given.
    pub async fn start(self, tmux: &Tmux, command: &str, mark: Option<&str>) -> Result<(), String> {
        let scope = tmux.pane_scope();
        let id = self.pane_id.as_str();
        let mut commands = Vec::new();
        if let Some(mark) = mark {
            commands.extend(mark_args(scope, id, mark, self.skip, None));
        }
        match self.kind {
            MoveKind::Respawn => {
                commands.push(vec![
                    "respawn-pane",
                    "-t",
                    id,
                    "-c",
                    self.cwd.as_str(),
                    "--",
                    command,
                ]);
                let started = tmux.run(chain(commands)).await;
                // On its own: an error in a command list stops the commands
                // after it, which would leave remain-on-exit on.
                if let Err(error) = tmux.run(&self.restore).await {
                    tracing::warn!(pane = %id, error = %error.sentence(), "remain-on-exit not put back");
                }
                started.map(drop).map_err(|error| {
                    format!(
                        "Claude Code stopped but did not start again: {}",
                        error.sentence()
                    )
                })
            }
            MoveKind::Shell => {
                // The shell must still be the one in front, or the keys
                // would go to whatever was started since.
                let still_shell = pane_field(tmux, id, "#{pane_current_command}")
                    .await
                    .is_ok_and(|now| super::panes::is_shell_name(&now));
                if !still_shell {
                    return Err(SHELL_NOT_BACK.to_string());
                }
                commands.extend(shell_keys(id, command));
                tmux.run(chain(commands)).await.map(drop).map_err(|error| {
                    format!(
                        "Claude Code stopped but the shell did not get the command: {}",
                        error.sentence()
                    )
                })
            }
        }
    }

    /// Start Claude Code again where it ran, resuming its conversation,
    /// its marks as they were.
    pub async fn start_where_it_was(self, tmux: &Tmux) -> Result<(), String> {
        let stay = self.stay.clone();
        self.start(tmux, &stay, None).await
    }

    /// Make `place`'s folder see the conversation, off the async threads.
    pub async fn share_with(&self, accounts: &Accounts, place: &Place) -> Result<bool, String> {
        let stores = accounts
            .stores()
            .ok_or_else(|| "No home folder for Claude Code accounts.".to_string())?;
        share(&self.record, stores.dir(place), &self.session).await
    }
}

/// Move `pane` (as a fresh collection saw it) to `place` now, marking it
/// `mark` (`global` or an id), for account `to`. The caller checked it is
/// movable; whether it is still idle is checked here, right before Claude
/// Code is stopped ([`NOT_IDLE`] when it is not, with nothing changed).
pub async fn move_now(
    tmux: &Tmux,
    accounts: &Accounts,
    pane: &PaneState,
    to: &str,
    mark: &str,
    place: &Place,
) -> Result<(), String> {
    if !accounts.is_signed_in(to) {
        return Err(not_signed_in(accounts, to));
    }
    // The command first, so nothing stops when it can't be built.
    let session = pane
        .record
        .as_ref()
        .and_then(|record| record.session_id.clone())
        .ok_or(NO_SESSION)?;
    let launch = accounts.restart_command(place, pane.info.skip_permissions, Some(&session))?;
    let stopped = stop(tmux, accounts, pane).await?;
    tracing::info!(pane = %pane.pane_id, to, mark, "moving Claude Code to another account");
    match stopped.share_with(accounts, place).await {
        Ok(_) => stopped.start(tmux, &launch, Some(mark)).await?,
        // The conversation stays where it was, and so does Claude Code.
        Err(error) => {
            let started = stopped.start_where_it_was(tmux).await;
            return Err(match started {
                Ok(()) => format!("{error} Claude Code started again on its account."),
                Err(start_error) => format!("{error} {start_error}"),
            });
        }
    }
    // Pinned to an account on the slot while another one was made global
    // meanwhile: it goes on to its folder once the pending round finds it
    // idle.
    let id = pane.pane_id.as_str();
    if *place == Place::Slot && mark != GLOBAL {
        let global = accounts.registry().global;
        if global != mark
            && set_marks(tmux, id, mark, pane.info.skip_permissions, Some(mark))
                .await
                .is_ok()
        {
            accounts.note_pending();
        }
    }
    // Claude Code may renew `to`'s token: read it again on the next load,
    // while `to` still counts as signed in for the next pane.
    accounts.reread_credentials(to);
    accounts.clear_move_error(id);
    Ok(())
}

/// Claude Code in respawned pane `pane` got SIGTERM but did not end in
/// time. Once it does (the pane dead, still the same pane program), it
/// starts again with `stay` (its own account, resuming its conversation);
/// then `restore` puts remain-on-exit back. A pane respawned meanwhile only
/// gets remain-on-exit back; a pane closed needs nothing, and a pane with
/// this id on a tmux server started since is left alone. Watched for
/// [`LATE_STOP_WAIT`] at most.
fn after_late_stop(
    guard: MoveGuard,
    tmux: Tmux,
    pane: String,
    pane_pid: u32,
    cwd: String,
    stay: String,
    restore: Vec<String>,
) {
    tokio::spawn(async move {
        // No other move of this pane until this one is over.
        let _guard = guard;
        let deadline = Instant::now() + LATE_STOP_WAIT;
        // The server this move ran on: a pane with this id on another one (a
        // tmux server started since) is not this move's to touch.
        let server = tmux.run(["display-message", "-p", "#{pid}"]).await.ok();
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let fields = match tmux
                .pane_fields(&pane, &["#{pane_pid}", "#{pane_dead}", "#{pid}"])
                .await
            {
                Ok(fields) => fields,
                // Gone (or tmux would not answer): put it back anyway, which
                // does nothing to a pane that no longer exists.
                Err(_) => {
                    let _ = tmux.run(&restore).await;
                    return;
                }
            };
            let pid = fields.first().map(String::as_str).unwrap_or_default();
            let dead = fields.get(1).map(String::as_str) == Some("1");
            let same_server = server
                .as_deref()
                .is_none_or(|server| Some(server.trim()) == fields.get(2).map(String::as_str));
            if !same_server {
                return;
            }
            // tmux 3.8 clears a dead pane's pid; before, it keeps the last.
            let same = pid == pane_pid.to_string() || (dead && pid.is_empty());
            if !same {
                // Respawned by someone else: nothing more to start, but
                // remain-on-exit was this move's, and goes back.
                if let Err(error) = tmux.run(&restore).await {
                    tracing::warn!(%pane, error = %error.sentence(), "remain-on-exit not put back");
                }
                return;
            }
            if dead {
                let respawn = [
                    "respawn-pane",
                    "-t",
                    pane.as_str(),
                    "-c",
                    cwd.as_str(),
                    "--",
                    stay.as_str(),
                ];
                match tmux.run(respawn).await {
                    Ok(_) => {
                        tracing::info!(%pane, "Claude Code ended late; started again on its account")
                    }
                    Err(error) => {
                        tracing::warn!(%pane, error = %error.sentence(), "Claude Code ended late and was not started again")
                    }
                }
            } else if Instant::now() < deadline {
                continue;
            }
            if let Err(error) = tmux.run(&restore).await {
                tracing::warn!(%pane, error = %error.sentence(), "remain-on-exit not put back");
            }
            return;
        }
    });
}

/// "<name> is not signed in", for account `id`.
pub fn not_signed_in(accounts: &Accounts, id: &str) -> String {
    let name = accounts
        .registry()
        .get(id)
        .map(|entry| entry.name.clone())
        .unwrap_or_else(|| id.to_string());
    format!("{name} is not signed in; sign it in first (tmux::accounts::login).")
}

/// `pane`'s account mark now (`None` unmarked), read afresh: a switch
/// takes seconds per pane, and a pane may be pinned meanwhile.
async fn mark_now(tmux: &Tmux, pane: &str) -> Result<Option<String>, String> {
    let fields = tmux.pane_fields(pane, &["#{@iii_claude_account}"]).await?;
    Ok(fields
        .first()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

/// `pane`'s marks now, read afresh (see [`mark_now`]).
pub async fn marks_now(tmux: &Tmux, pane: &str) -> Result<super::panes::Marks, String> {
    let fields = tmux
        .pane_fields(
            pane,
            &[
                "#{@iii_claude_account}",
                "#{@iii_skip_permissions}",
                "#{@iii_claude_pending}",
            ],
        )
        .await?;
    let field = |at: usize| fields.get(at).map(String::as_str).unwrap_or_default();
    Ok(super::panes::Marks::from_raw(
        field(0),
        field(1).trim() == "1",
        field(2),
    ))
}

/// Mark `pane` to move to `to` once its turn ends.
async fn wait_for_turn(
    tmux: &Tmux,
    accounts: &Accounts,
    pane: &PaneState,
    mark: &str,
    to: &str,
) -> Result<(), String> {
    set_marks(
        tmux,
        &pane.pane_id,
        mark,
        pane.info.skip_permissions,
        Some(to),
    )
    .await?;
    accounts.note_waiting(&pane.pane_id, pane.claude_pid);
    Ok(())
}

/// Carry out `plan` on `pane`. Returns the state `tmux::panes::account`
/// reports. A pane that began a turn since its plan was made waits for it
/// (`after_turn`).
pub async fn apply(
    tmux: &Tmux,
    accounts: &Accounts,
    pane: &PaneState,
    plan: &Plan,
) -> Result<&'static str, String> {
    let skip = pane.info.skip_permissions;
    // A move under way sets the marks itself when it ends: a change now
    // would be overwritten, or the move would go on to where it was going.
    if accounts.moving.is_busy(&pane.pane_id) {
        return Err(BUSY.to_string());
    }
    accounts.clear_move_error(&pane.pane_id);
    match plan {
        Plan::Marks { mark, state, .. } => {
            set_marks(tmux, &pane.pane_id, mark, skip, None).await?;
            accounts.forget_waiting(&pane.pane_id);
            Ok(state)
        }
        Plan::AfterTurn { mark, to, .. } => {
            wait_for_turn(tmux, accounts, pane, mark, to).await?;
            Ok("after_turn")
        }
        Plan::Now { mark, to, place } => {
            match move_now(tmux, accounts, pane, to, mark, place).await {
                Ok(()) => {
                    accounts.forget_waiting(&pane.pane_id);
                    Ok("moved")
                }
                Err(error) if error == NOT_IDLE => {
                    wait_for_turn(tmux, accounts, pane, mark, to).await?;
                    Ok("after_turn")
                }
                Err(error) => Err(error),
            }
        }
    }
}

/// The account the waiting move of `pane` goes to: its mark's account (the
/// global one for `global`), else the pending mark's.
pub fn pending_target(pane: &PaneState, registry: &Registry) -> Option<String> {
    let pending = pane.marks.pending.as_ref()?;
    let mark = pane.marks.account.as_ref().unwrap_or(pending);
    Some(account_for(mark, registry))
}

/// What to do about a pane carrying `@iii_claude_pending` (§3.7): `None`
/// to leave it, `Ok(plan)` (a [`Plan::Marks`] clears the mark), or
/// `Err(why)` when the move can't happen any more: the mark is cleared and
/// the reason shown on the pane.
pub fn pending_plan(
    pane: &PaneState,
    registry: &Registry,
    signed_in: impl Fn(&str) -> bool,
) -> Option<Result<Plan, String>> {
    let pending = pane.marks.pending.clone()?;
    let mark = pane
        .marks
        .account
        .clone()
        .unwrap_or_else(|| pending.clone());
    let to = account_for(&mark, registry);
    let place = place_for(&mark, registry);
    let settled = Plan::Marks {
        mark: mark.clone(),
        to: to.clone(),
        state: "cancelled",
    };
    // Gone, or it already runs there.
    if !pane.running() || pane.place.as_ref() == Some(&place) {
        return Some(Ok(settled));
    }
    if !registry.has(&to) {
        return Some(Err(super::manage::no_account(&to)));
    }
    if pane.marks_shared {
        return Some(Err(super::panes::SHARED_MARKS.to_string()));
    }
    if !pane.info.movable || pane.kind.is_none() {
        return Some(Err(pane.why_not_movable()));
    }
    if !pane.idle() {
        // The global account changed again while it waited: say where to.
        return (pending != to).then_some(Ok(Plan::AfterTurn { mark, to, place }));
    }
    if !signed_in(&to) {
        return Some(Err(format!(
            "{} is not signed in; sign it in first (tmux::accounts::login).",
            registry.get(&to).map_or(to.as_str(), |e| e.name.as_str())
        )));
    }
    Some(Ok(Plan::Now { mark, to, place }))
}

/// Move the idle Claude Code panes using `from` to `to`, pinning them
/// there: those that follow the global account, are pinned to `from` or
/// were started by hand; never those pinned elsewhere. Each account's
/// panes are where its sign-in is: the slot for the global one, its folder
/// otherwise. A pane that began a turn meanwhile stays.
pub async fn move_idle(
    tmux: &Tmux,
    accounts: &Accounts,
    panes: &std::collections::HashMap<String, PaneState>,
    from: &str,
    to: &str,
) -> Result<Vec<String>, String> {
    let registry = accounts.registry();
    for id in [from, to] {
        if !registry.has(id) {
            return Err(super::manage::no_account(id));
        }
    }
    if from == to {
        return Err("Pick another account to move them to.".to_string());
    }
    if !accounts.is_signed_in(to) {
        return Err(not_signed_in(accounts, to));
    }
    let from_place = Accounts::place_of(&registry, from);
    let to_place = Accounts::place_of(&registry, to);
    let mut candidates: Vec<&PaneState> = panes
        .values()
        .filter(|pane| pane.running() && pane.place.as_ref() == Some(&from_place))
        .filter(|pane| {
            matches!(pane.marks.account.as_deref(), None | Some(GLOBAL))
                || pane.marks.account.as_deref() == Some(from)
        })
        .filter(|pane| pane.idle() && pane.can_move())
        .collect();
    candidates.sort_by(|a, b| a.pane_id.cmp(&b.pane_id));
    let mut moved = Vec::new();
    for pane in candidates {
        // Pinned elsewhere meanwhile: no longer one of these.
        match mark_now(tmux, &pane.pane_id).await {
            Ok(None) => {}
            Ok(Some(mark)) if mark == GLOBAL || mark == from => {}
            Ok(Some(_)) => continue,
            Err(error) => {
                accounts.note_move_error(&pane.pane_id, &error);
                continue;
            }
        }
        match move_now(tmux, accounts, pane, to, to, &to_place).await {
            Ok(()) => moved.push(pane.pane_id.clone()),
            Err(error) if error == NOT_IDLE => {}
            Err(error) => {
                tracing::warn!(pane = %pane.pane_id, %error, "pane not moved");
                accounts.note_move_error(&pane.pane_id, &error);
            }
        }
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::panes::Marks;
    use crate::activity::ClaudeRecord;
    use crate::model::{AgentState, Follow, PaneClaude};

    /// A pane on `account`'s place in a registry whose global is `default`
    /// unless the test says otherwise: `default` and `global` mean the slot.
    fn pane(account: &str, mark: Option<&str>, pending: Option<&str>, status: &str) -> PaneState {
        let place = if account == "default" {
            Place::Slot
        } else {
            Place::Folder(account.to_string())
        };
        PaneState {
            pane_id: "%3".into(),
            pane_pid: 10,
            claude_pid: Some(10),
            record: Some(ClaudeRecord {
                config_dir: PathBuf::from("/h/.claude"),
                session_id: Some("s1".into()),
                status: Some(status.into()),
                cwd: Some("/p".into()),
            }),
            marks: Marks {
                account: mark.map(String::from),
                skip: false,
                pending: pending.map(String::from),
            },
            info: PaneClaude {
                account: Some(account.into()),
                config_dir: None,
                follow: Follow::Global,
                pending: pending.map(String::from),
                skip_permissions: false,
                session_id: Some("s1".into()),
                movable: true,
                not_movable_reason: None,
                move_error: None,
            },
            agent_state: Some(AgentState::Idle),
            cwd: "/p".into(),
            place: Some(place),
            kind: Some(MoveKind::Respawn),
            marks_shared: false,
        }
    }

    fn registry(global: &str) -> Registry {
        let mut registry = Registry::default();
        for id in ["personal", "side"] {
            registry.accounts.push(crate::accounts::registry::Entry {
                id: id.into(),
                name: id.into(),
                color: "green".into(),
                shared: vec![],
                created_at_ms: 1,
            });
        }
        registry.global = global.into();
        registry
    }

    #[test]
    fn plans_follow_d7_and_d9() {
        let reg = registry("default");
        // Pin to where it runs: marks only.
        assert_eq!(
            plan(
                &pane("default", Some("global"), None, "idle"),
                &reg,
                "default"
            )
            .unwrap(),
            Plan::Marks {
                mark: "default".into(),
                to: "default".into(),
                state: "pinned"
            }
        );
        // Follow the global one it already runs on.
        assert!(matches!(
            plan(&pane("default", None, None, "busy"), &reg, "global").unwrap(),
            Plan::Marks {
                state: "following",
                ..
            }
        ));
        // Continue in X: now when idle, after the turn otherwise.
        assert_eq!(
            plan(
                &pane("default", Some("global"), None, "idle"),
                &reg,
                "personal"
            )
            .unwrap(),
            Plan::Now {
                mark: "personal".into(),
                to: "personal".into(),
                place: Place::Folder("personal".into())
            }
        );
        assert_eq!(
            plan(
                &pane("default", Some("global"), None, "busy"),
                &reg,
                "personal"
            )
            .unwrap(),
            Plan::AfterTurn {
                mark: "personal".into(),
                to: "personal".into(),
                place: Place::Folder("personal".into())
            }
        );
        // `shell` (a background command) and `waiting` wait too.
        assert!(matches!(
            plan(&pane("default", None, None, "shell"), &reg, "side").unwrap(),
            Plan::AfterTurn { .. }
        ));
        // Follow the global one from another account: a move to the slot.
        assert_eq!(
            plan(&pane("side", Some("side"), None, "idle"), &reg, "global").unwrap(),
            Plan::Now {
                mark: "global".into(),
                to: "default".into(),
                place: Place::Slot
            }
        );
        // Pinned to the global account from a folder: on the slot, pinned.
        assert_eq!(
            plan(&pane("side", None, None, "idle"), &reg, "default").unwrap(),
            Plan::Now {
                mark: "default".into(),
                to: "default".into(),
                place: Place::Slot
            }
        );
        // A pane on the slot pinned to the global account: marks only.
        assert!(matches!(
            plan(&pane("default", None, None, "busy"), &reg, "default").unwrap(),
            Plan::Marks {
                state: "pinned",
                ..
            }
        ));
        // Cancel: back to the current account with a move pending.
        assert!(matches!(
            plan(
                &pane("default", Some("global"), Some("personal"), "busy"),
                &reg,
                "default"
            )
            .unwrap(),
            Plan::Marks {
                state: "cancelled",
                ..
            }
        ));
        // Not movable, unknown account.
        let mut nested = pane("default", None, None, "idle");
        nested.kind = None;
        nested.info.movable = false;
        nested.info.not_movable_reason = Some("nested".into());
        assert_eq!(plan(&nested, &reg, "personal"), Err("nested".to_string()));
        assert!(plan(&nested, &reg, "nobody").is_err());
        // Not running: marks only.
        let mut stopped = pane("default", None, None, "idle");
        stopped.claude_pid = None;
        assert!(matches!(
            plan(&stopped, &reg, "personal").unwrap(),
            Plan::Marks {
                state: "pinned",
                ..
            }
        ));
    }

    #[test]
    fn pending_moves_wait_for_idle_and_follow_the_global() {
        let reg = registry("default");
        let all = |_: &str| true;
        // On the side folder, moving to the slot (following) after its turn.
        let busy = pane("side", Some("global"), Some("default"), "busy");
        assert_eq!(pending_plan(&busy, &reg, all), None, "still working");
        let idle = pane("side", Some("global"), Some("default"), "idle");
        assert_eq!(
            pending_plan(&idle, &reg, all),
            Some(Ok(Plan::Now {
                mark: "global".into(),
                to: "default".into(),
                place: Place::Slot
            }))
        );
        // The global one is `personal` now: still the slot, a new pending.
        let reg_personal = registry("personal");
        assert_eq!(
            pending_plan(&busy, &reg_personal, all),
            Some(Ok(Plan::AfterTurn {
                mark: "global".into(),
                to: "personal".into(),
                place: Place::Slot
            }))
        );
        // Pinned to the old global on the slot (G4): its folder after the turn.
        let pinned = pane("default", Some("default"), Some("default"), "idle");
        assert_eq!(
            pending_plan(&pinned, &reg_personal, all),
            Some(Ok(Plan::Now {
                mark: "default".into(),
                to: "default".into(),
                place: Place::Folder("default".into())
            }))
        );
        // And back to where it runs: nothing to move.
        assert!(matches!(
            pending_plan(&pinned, &reg, all),
            Some(Ok(Plan::Marks {
                state: "cancelled",
                ..
            }))
        ));
        assert_eq!(
            pending_plan(&pane("side", Some("global"), None, "idle"), &reg, all),
            None
        );
        // Without a raw status it is not idle (D7), whatever the screen says.
        let mut unknown = idle.clone();
        unknown.record.as_mut().unwrap().status = None;
        assert!(!unknown.idle());
        assert_eq!(pending_plan(&unknown, &reg, all), None);
    }

    #[test]
    fn pending_moves_that_cant_happen_say_why() {
        let reg = registry("personal");
        let idle = pane("side", Some("global"), Some("personal"), "idle");
        // The target signed out meanwhile.
        let refused = pending_plan(&idle, &reg, |_| false).unwrap().unwrap_err();
        assert!(refused.contains("not signed in"), "{refused}");
        // No longer movable.
        let mut nested = idle.clone();
        nested.info.movable = false;
        nested.info.not_movable_reason = Some("nested".into());
        assert_eq!(
            pending_plan(&nested, &reg, |_| true),
            Some(Err("nested".to_string()))
        );
        // The account went away.
        let mut gone = idle.clone();
        gone.marks.account = Some("ghost".into());
        assert!(pending_plan(&gone, &reg, |_| true).unwrap().is_err());
        // tmux before 3.0, two Claude Code panes in its window.
        let mut shared = idle.clone();
        shared.marks_shared = true;
        assert_eq!(
            pending_plan(&shared, &reg, |_| true),
            Some(Err(crate::accounts::panes::SHARED_MARKS.to_string()))
        );
        assert_eq!(
            plan(&shared, &reg, "default"),
            Err(crate::accounts::panes::SHARED_MARKS.to_string()),
            "not even its marks change"
        );
    }

    #[test]
    fn the_shell_line_is_cleared_whole_before_the_command() {
        assert_eq!(
            chain(shell_keys(
                "%4",
                "env CLAUDE_CONFIG_DIR=/a claude --resume s1"
            )),
            vec![
                "send-keys",
                "-t",
                "%4",
                "C-e",
                ";",
                "send-keys",
                "-t",
                "%4",
                "C-u",
                ";",
                "send-keys",
                "-t",
                "%4",
                "-l",
                "env CLAUDE_CONFIG_DIR=/a claude --resume s1",
                ";",
                "send-keys",
                "-t",
                "%4",
                "Enter"
            ]
        );
    }

    #[test]
    fn options_go_back_as_they_were() {
        assert_eq!(
            restore_option("-p", "%1", "remain-on-exit", None),
            vec!["set-option", "-p", "-u", "-t", "%1", "remain-on-exit"],
            "unset stays unset"
        );
        assert_eq!(
            restore_option("-w", "%1", "remain-on-exit", Some("failed")),
            vec!["set-option", "-w", "-t", "%1", "remain-on-exit", "failed"]
        );
    }

    #[test]
    fn transcripts_are_copied_unless_shared() {
        let base =
            std::env::temp_dir().join(format!("tmux-moves-{}", uuid::Uuid::new_v4().simple()));
        let from = base.join("home/.claude");
        let to = base.join("accounts/personal");
        let project = from.join("projects/-home-dev-app");
        std::fs::create_dir_all(project.join("s1/subagents")).unwrap();
        std::fs::write(project.join("s1.jsonl"), "{\"a\":1}\n").unwrap();
        std::fs::write(project.join("s1/subagents/agent-x.jsonl"), "{}\n").unwrap();
        std::fs::create_dir_all(&to).unwrap();
        assert_eq!(
            find_transcript(&from, "s1", Some("/home/dev/app")),
            Some(project.join("s1.jsonl"))
        );
        assert_eq!(
            find_transcript(&from, "s1", Some("/elsewhere")),
            Some(project.join("s1.jsonl")),
            "found in any project folder"
        );
        assert!(share_transcript(&from, &to, "s1", Some("/home/dev/app")).unwrap());
        let copied = to.join("projects/-home-dev-app");
        assert_eq!(
            std::fs::read_to_string(copied.join("s1.jsonl")).unwrap(),
            "{\"a\":1}\n"
        );
        assert!(copied.join("s1/subagents/agent-x.jsonl").is_file());
        // A newer turn is copied over the older copy.
        std::fs::write(project.join("s1.jsonl"), "{\"a\":1}\n{\"a\":2}\n").unwrap();
        assert!(share_transcript(&from, &to, "s1", None).unwrap());
        assert_eq!(
            std::fs::read_to_string(copied.join("s1.jsonl")).unwrap(),
            "{\"a\":1}\n{\"a\":2}\n"
        );
        // The conversation went on in the other account: never overwritten.
        std::fs::write(copied.join("s1.jsonl"), "{\"a\":1}\n{\"a\":2}\n{\"b\":3}\n").unwrap();
        assert_eq!(
            share_transcript(&from, &to, "s1", None),
            Err(NEWER_COPY.to_string())
        );
        assert_eq!(
            std::fs::read_to_string(copied.join("s1.jsonl")).unwrap(),
            "{\"a\":1}\n{\"a\":2}\n{\"b\":3}\n"
        );
        // Shared `projects/`: the same file, nothing copied.
        let shared = base.join("accounts/side");
        std::fs::create_dir_all(&shared).unwrap();
        std::os::unix::fs::symlink(from.join("projects"), shared.join("projects")).unwrap();
        assert!(!share_transcript(&from, &shared, "s1", None).unwrap());
        assert_eq!(
            share_transcript(&from, &to, "nope", None),
            Err(NO_TRANSCRIPT.to_string())
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn marks_set_and_clear() {
        assert_eq!(
            chain(mark_args("-p", "%1", "personal", true, Some("side"))),
            vec![
                "set-option",
                "-p",
                "-t",
                "%1",
                "@iii_claude_account",
                "personal",
                ";",
                "set-option",
                "-p",
                "-t",
                "%1",
                "@iii_skip_permissions",
                "1",
                ";",
                "set-option",
                "-p",
                "-t",
                "%1",
                "@iii_claude_pending",
                "side"
            ]
        );
        let cleared = chain(mark_args("-w", "%1", "global", false, None));
        assert!(cleared.windows(6).any(|w| w
            == [
                "set-option",
                "-w",
                "-u",
                "-t",
                "%1",
                "@iii_skip_permissions"
            ]));
        assert!(cleared
            .windows(6)
            .any(|w| w == ["set-option", "-w", "-u", "-t", "%1", "@iii_claude_pending"]));
    }

    #[test]
    fn one_move_at_a_time() {
        let moving = Moving::default();
        let first = moving.begin("%1").unwrap();
        assert!(moving.begin("%1").is_none());
        assert!(moving.begin("%2").is_some());
        drop(first);
        assert!(moving.begin("%1").is_some());
    }
}
