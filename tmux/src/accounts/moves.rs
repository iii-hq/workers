//! Moving a Claude Code pane to another account (agent-accounts D6–D9,
//! §3.7): Claude Code stops, and starts again on the other account's folder
//! with `--resume <session>`, after the worker made sure that folder sees
//! the conversation.
//!
//! - A pane whose program is Claude Code (the worker started it, or
//!   `sh -c` did) is kept with remain-on-exit while Claude Code ends on
//!   SIGTERM, then respawned with the new command; its remain-on-exit is put
//!   back as it was. Stopping it first, instead of `respawn-pane -k`, lets
//!   Claude Code write the end of its conversation before the new one reads
//!   it.
//! - Claude Code started by hand in an interactive shell ends on SIGTERM;
//!   the shell gets the command typed (`C-u`, the command, Enter). It is
//!   always the explicit form there (`env CLAUDE_CONFIG_DIR=…`): the shell
//!   kept the environment it started with (D3).
//!
//! Claude Code's own shutdown can take more than D6's five seconds (its
//! failsafe waits at least five, plus SessionEnd hooks), so the worker
//! waits up to ten; if it still runs, nothing else is changed.
//!
//! A move happens only between turns (D7); otherwise the pane gets
//! `@iii_claude_pending` and the worker moves it when it is idle.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::launch::{MARK_ACCOUNT, MARK_PENDING, MARK_SKIP};
use super::panes::{MoveKind, PaneState, NOT_RUNNING, NO_SESSION};
use super::registry::{self, Registry};
use super::{tmux_env, Accounts, GLOBAL};
use crate::tmux_cli::Tmux;

/// How long Claude Code gets to end on SIGTERM.
pub const STOP_WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(100);

pub const DID_NOT_STOP: &str =
    "Claude Code did not stop within ten seconds; the pane was left as it was.";
pub const NO_TRANSCRIPT: &str = "Claude Code's conversation file was not found.";
pub const BUSY: &str = "This pane is being moved already.";

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
    /// Move it now (it is idle).
    Now { mark: String, to: String },
    /// Move it once its turn ends.
    AfterTurn { mark: String, to: String },
}

/// The plan for `want` (`global` or an id) on `pane`, `registry` naming
/// the accounts and the global one.
pub fn plan(pane: &PaneState, registry: &Registry, want: &str) -> Result<Plan, String> {
    let want = want.trim();
    if want != GLOBAL && !registry.has(want) {
        return Err(super::manage::no_account(want));
    }
    let mark = want.to_string();
    let to = if want == GLOBAL {
        registry.global.clone()
    } else {
        mark.clone()
    };
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
    if pane.claude_pid.is_none() {
        return Ok(settled(false));
    }
    if pane.info.account.as_deref() == Some(to.as_str()) {
        return Ok(settled(pane.marks.pending.is_some()));
    }
    if !pane.info.movable || pane.kind.is_none() {
        return Err(pane
            .info
            .not_movable_reason
            .clone()
            .unwrap_or_else(|| "Claude Code in this pane can't be moved.".to_string()));
    }
    Ok(if pane.idle() {
        Plan::Now { mark, to }
    } else {
        Plan::AfterTurn { mark, to }
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
        } else if kind.is_file() {
            copy_file(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Make account folder `to_dir` see session `id`'s conversation, which
/// lives under `from_dir`: unless it is already the same file (both share
/// `projects/`), copy `<id>.jsonl` and its `<id>/` folder into the same
/// project folder there. Returns whether it copied.
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
pub struct Moving(std::sync::Mutex<HashSet<String>>);

pub struct MoveGuard<'a> {
    moving: &'a Moving,
    pane: String,
}

impl Moving {
    pub fn begin(&self, pane: &str) -> Option<MoveGuard<'_>> {
        let mut set = self.0.lock().unwrap_or_else(|p| p.into_inner());
        set.insert(pane.to_string()).then(|| MoveGuard {
            moving: self,
            pane: pane.to_string(),
        })
    }
}

impl Drop for MoveGuard<'_> {
    fn drop(&mut self) {
        self.moving
            .0
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

/// Move `pane` (as a fresh collection saw it) to account `to` now, marking
/// it `mark` (`global` or `to`). The caller checked it is idle and movable.
pub async fn move_now(
    tmux: &Tmux,
    accounts: &Accounts,
    pane: &PaneState,
    to: &str,
    mark: &str,
) -> Result<(), String> {
    let _guard = accounts.moving.begin(&pane.pane_id).ok_or(BUSY)?;
    let paths = accounts
        .paths()
        .cloned()
        .ok_or_else(|| "No home folder for Claude Code accounts.".to_string())?;
    let claude_pid = pane.claude_pid.ok_or(NOT_RUNNING)?;
    let kind = pane.kind.ok_or_else(|| {
        pane.info
            .not_movable_reason
            .clone()
            .unwrap_or_else(|| "Claude Code in this pane can't be moved.".to_string())
    })?;
    let record = pane.record.clone().ok_or(NO_SESSION)?;
    let session = record.session_id.clone().ok_or(NO_SESSION)?;
    let skip = pane.info.skip_permissions;

    // The command first, so nothing stops when it can't be built.
    let launch_account = if kind == MoveKind::Respawn && mark == GLOBAL {
        GLOBAL
    } else {
        to
    };
    let launch = accounts.plan_launch(launch_account, skip, Some(&session))?;
    if launch_account == GLOBAL {
        tmux_env::reconcile(tmux, accounts).await?;
    }
    {
        let (from_dir, to_dir) = (record.config_dir.clone(), paths.account_dir(to));
        let (id, cwd) = (session.clone(), record.cwd.clone());
        tokio::task::spawn_blocking(move || {
            share_transcript(&from_dir, &to_dir, &id, cwd.as_deref())
        })
        .await
        .map_err(|error| format!("The worker failed to copy the conversation ({error})."))??;
    }
    let cwd = record
        .cwd
        .clone()
        .filter(|cwd| Path::new(cwd).is_dir())
        .unwrap_or_else(|| pane.cwd.clone());
    let scope = tmux.pane_scope();
    let id = pane.pane_id.as_str();
    tracing::info!(pane = %id, to, mark, ?kind, "moving Claude Code to another account");

    match kind {
        MoveKind::Respawn => {
            let before = tmux
                .run(["show-options", scope, "-v", "-t", id, "remain-on-exit"])
                .await
                .unwrap_or_default()
                .trim()
                .to_string();
            tmux.run(["set-option", scope, "-t", id, "remain-on-exit", "on"])
                .await
                .map_err(|error| error.sentence())?;
            let restore: Vec<&str> = if before.is_empty() {
                vec!["set-option", scope, "-u", "-t", id, "remain-on-exit"]
            } else {
                vec![
                    "set-option",
                    scope,
                    "-t",
                    id,
                    "remain-on-exit",
                    before.as_str(),
                ]
            };
            signal(claude_pid, libc::SIGTERM);
            let dead = wait_for(STOP_WAIT, || async {
                pane_field(tmux, id, "#{pane_dead}").await.as_deref() == Ok("1")
            })
            .await;
            if !dead {
                let _ = tmux.run(&restore).await;
                return Err(DID_NOT_STOP.to_string());
            }
            let cwd = crate::tmux_cli::format_literal(&cwd);
            let mut commands = mark_args(scope, id, mark, skip, None);
            commands.push(vec![
                "respawn-pane",
                "-t",
                id,
                "-c",
                cwd.as_str(),
                "--",
                launch.command.as_str(),
            ]);
            commands.push(restore.clone());
            tmux.run(chain(commands)).await.map_err(|error| {
                format!(
                    "Claude Code stopped but did not start again on the account: {}",
                    error.sentence()
                )
            })?;
        }
        MoveKind::Shell => {
            signal(claude_pid, libc::SIGTERM);
            let gone = wait_for(STOP_WAIT, || async { !alive(claude_pid) }).await;
            if !gone {
                return Err(DID_NOT_STOP.to_string());
            }
            // The shell takes the terminal back as soon as its job ends.
            let _ = wait_for(Duration::from_secs(2), || async {
                pane_field(tmux, id, "#{pane_current_command}")
                    .await
                    .is_ok_and(|now| super::panes::is_shell_name(&now))
            })
            .await;
            let explicit = accounts.plan_launch(to, skip, Some(&session))?;
            let mut commands = mark_args(scope, id, mark, skip, None);
            commands.push(vec!["send-keys", "-t", id, "C-u"]);
            commands.push(vec!["send-keys", "-t", id, "-l", explicit.command.as_str()]);
            commands.push(vec!["send-keys", "-t", id, "Enter"]);
            tmux.run(chain(commands)).await.map_err(|error| {
                format!(
                    "Claude Code stopped but the shell did not get the command: {}",
                    error.sentence()
                )
            })?;
        }
    }
    accounts.forget_credentials(to);
    Ok(())
}

/// Carry out `plan` on `pane`. Returns the state `tmux::panes::account`
/// reports.
pub async fn apply(
    tmux: &Tmux,
    accounts: &Accounts,
    pane: &PaneState,
    plan: &Plan,
) -> Result<&'static str, String> {
    let skip = pane.info.skip_permissions;
    match plan {
        Plan::Marks { mark, state, .. } => {
            set_marks(tmux, &pane.pane_id, mark, skip, None).await?;
            Ok(state)
        }
        Plan::AfterTurn { mark, to } => {
            set_marks(tmux, &pane.pane_id, mark, skip, Some(to)).await?;
            accounts.note_pending();
            Ok("after_turn")
        }
        Plan::Now { mark, to } => {
            move_now(tmux, accounts, pane, to, mark).await?;
            Ok("moved")
        }
    }
}

/// What to do about a pane carrying `@iii_claude_pending` (§3.7): `None`
/// to leave it, else the plan (a [`Plan::Marks`] clears the mark).
pub fn pending_plan(pane: &PaneState, registry: &Registry) -> Option<Plan> {
    let pending = pane.marks.pending.clone()?;
    let mark = pane
        .marks
        .account
        .clone()
        .unwrap_or_else(|| pending.clone());
    let to = if mark == GLOBAL {
        registry.global.clone()
    } else {
        mark.clone()
    };
    let settled = Plan::Marks {
        mark: mark.clone(),
        to: to.clone(),
        state: "cancelled",
    };
    // Gone, or the account went away, or it already runs there.
    if pane.claude_pid.is_none()
        || !registry.has(&to)
        || pane.info.account.as_deref() == Some(to.as_str())
        || pane.kind.is_none()
    {
        return Some(settled);
    }
    if !pane.idle() {
        // The global account changed again while it waited: say where to.
        return (pending != to).then_some(Plan::AfterTurn { mark, to });
    }
    Some(Plan::Now { mark, to })
}

/// How panes that follow the global account go along when it changes (D8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SwitchMode {
    /// Idle ones move now, working or waiting ones after their turn.
    #[default]
    IdleNowWorkingAfter,
    /// They stay, pinned to the account they run on.
    NewOnly,
}

/// What a switch of the global account did to the panes that follow it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Switched {
    pub moved: Vec<String>,
    pub after_turn: Vec<String>,
    pub stayed: Vec<String>,
}

/// Make `to` the global account (D3, D8): the registry, tmux's global
/// environment, then the panes that follow it, as `mode` says. `panes` is a
/// fresh collection.
pub async fn switch_global(
    tmux: &Tmux,
    accounts: &Accounts,
    panes: &std::collections::HashMap<String, PaneState>,
    to: &str,
    mode: SwitchMode,
) -> Result<Switched, String> {
    let to = to.trim();
    accounts.update_registry(|registry| {
        if !registry.has(to) {
            return Err(super::manage::no_account(to));
        }
        registry.global = to.to_string();
        Ok(())
    })?;
    if let Err(error) = tmux_env::reconcile(tmux, accounts).await {
        tracing::warn!(%error, "tmux global CLAUDE_CONFIG_DIR not set");
    }
    let mut out = Switched::default();
    let mut following: Vec<&PaneState> = panes
        .values()
        .filter(|pane| pane.marks.account.as_deref() == Some(GLOBAL))
        .filter(|pane| pane.claude_pid.is_some())
        .filter(|pane| pane.info.account.as_deref() != Some(to))
        .collect();
    following.sort_by(|a, b| a.pane_id.cmp(&b.pane_id));
    for pane in following {
        let id = pane.pane_id.clone();
        let skip = pane.info.skip_permissions;
        match mode {
            SwitchMode::NewOnly => {
                // Pinned where it runs, so its chip never says the new one.
                if let Some(current) = pane.info.account.as_deref() {
                    set_marks(tmux, &id, current, skip, None).await?;
                }
                out.stayed.push(id);
            }
            SwitchMode::IdleNowWorkingAfter => {
                if pane.kind.is_none() || !pane.info.movable {
                    out.stayed.push(id);
                } else if pane.idle() {
                    match move_now(tmux, accounts, pane, to, GLOBAL).await {
                        Ok(()) => out.moved.push(id),
                        Err(error) => {
                            tracing::warn!(pane = %id, %error, "pane not moved");
                            out.stayed.push(id);
                        }
                    }
                } else {
                    set_marks(tmux, &id, GLOBAL, skip, Some(to)).await?;
                    accounts.note_pending();
                    out.after_turn.push(id);
                }
            }
        }
    }
    Ok(out)
}

/// Move the idle Claude Code panes running on `from` to `to`, pinning them
/// there: those that follow the global account, are pinned to `from` or
/// were started by hand; never those pinned elsewhere.
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
    let mut candidates: Vec<&PaneState> = panes
        .values()
        .filter(|pane| pane.claude_pid.is_some() && pane.info.account.as_deref() == Some(from))
        .filter(|pane| {
            matches!(pane.marks.account.as_deref(), None | Some(GLOBAL))
                || pane.marks.account.as_deref() == Some(from)
        })
        .filter(|pane| pane.idle() && pane.info.movable && pane.kind.is_some())
        .collect();
    candidates.sort_by(|a, b| a.pane_id.cmp(&b.pane_id));
    let mut moved = Vec::new();
    for pane in candidates {
        match move_now(tmux, accounts, pane, to, to).await {
            Ok(()) => moved.push(pane.pane_id.clone()),
            Err(error) => tracing::warn!(pane = %pane.pane_id, %error, "pane not moved"),
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

    fn pane(account: &str, mark: Option<&str>, pending: Option<&str>, status: &str) -> PaneState {
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
            },
            agent_state: Some(AgentState::Idle),
            cwd: "/p".into(),
            kind: Some(MoveKind::Respawn),
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
                to: "personal".into()
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
                to: "personal".into()
            }
        );
        // `shell` (a background command) and `waiting` wait too.
        assert!(matches!(
            plan(&pane("default", None, None, "shell"), &reg, "side").unwrap(),
            Plan::AfterTurn { .. }
        ));
        // Follow the global one from another account: a move.
        assert_eq!(
            plan(&pane("side", Some("side"), None, "idle"), &reg, "global").unwrap(),
            Plan::Now {
                mark: "global".into(),
                to: "default".into()
            }
        );
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
        let reg = registry("personal");
        let busy = pane("default", Some("global"), Some("personal"), "busy");
        assert_eq!(pending_plan(&busy, &reg), None, "still working");
        let idle = pane("default", Some("global"), Some("personal"), "idle");
        assert_eq!(
            pending_plan(&idle, &reg),
            Some(Plan::Now {
                mark: "global".into(),
                to: "personal".into()
            })
        );
        // The global one moved on to `side` meanwhile.
        let reg_side = registry("side");
        assert_eq!(
            pending_plan(&busy, &reg_side),
            Some(Plan::AfterTurn {
                mark: "global".into(),
                to: "side".into()
            })
        );
        // And back to where it runs: nothing to move.
        let reg_default = registry("default");
        assert!(matches!(
            pending_plan(&busy, &reg_default),
            Some(Plan::Marks {
                state: "cancelled",
                ..
            })
        ));
        assert_eq!(
            pending_plan(&pane("default", Some("global"), None, "idle"), &reg),
            None
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
        std::fs::write(project.join("s1.jsonl"), "{\"a\":2}\n").unwrap();
        assert!(share_transcript(&from, &to, "s1", None).unwrap());
        assert_eq!(
            std::fs::read_to_string(copied.join("s1.jsonl")).unwrap(),
            "{\"a\":2}\n"
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
