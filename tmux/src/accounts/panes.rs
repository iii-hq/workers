//! Which account a pane's Claude Code runs on, how it follows the global
//! account, and whether the worker can move it (agent-accounts D4–D6).

use std::path::Path;

use super::registry::Registry;
use super::{Accounts, GLOBAL};
use crate::activity::ClaudeRecord;
use crate::model::{Follow, PaneClaude};
use crate::procs::{ProcInfo, ProcTable};

/// The account marks a pane carries (tmux user options, D4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks {
    /// `global` or an account id.
    pub account: Option<String>,
    pub skip: bool,
    pub pending: Option<String>,
}

impl Marks {
    pub fn from_raw(account: &str, skip: bool, pending: &str) -> Self {
        let text = |value: &str| Some(value.trim().to_string()).filter(|v| !v.is_empty());
        Self {
            account: text(account),
            skip,
            pending: text(pending),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.account.is_none() && !self.skip && self.pending.is_none()
    }

    pub fn follow(&self) -> Follow {
        match self.account.as_deref() {
            None => Follow::Manual,
            Some(GLOBAL) => Follow::Global,
            Some(_) => Follow::Pinned,
        }
    }
}

/// How a Claude Code pane is moved to another account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    /// Claude Code is the pane's program: `respawn-pane` starts it again.
    Respawn,
    /// Claude Code runs under the pane's interactive shell: it is stopped and
    /// the command typed into the shell.
    Shell,
}

pub const NO_SESSION: &str = "Claude Code has not recorded its session yet.";
pub const NESTED: &str =
    "Claude Code runs inside another program in this pane (ssh, docker, tmux, a script…).";
pub const NOT_RUNNING: &str = "Claude Code is not running in this pane.";

const SHELLS: [&str; 12] = [
    "sh", "bash", "zsh", "dash", "fish", "ksh", "mksh", "tcsh", "csh", "nu", "elvish", "xonsh",
];

/// `name` (a process or `#{pane_current_command}`) is a shell's.
pub fn is_shell_name(name: &str) -> bool {
    SHELLS.contains(&name.trim_start_matches('-'))
}

fn is_shell(proc: &ProcInfo) -> bool {
    proc.names().any(is_shell_name)
}

/// A shell started with `-c` runs one command and ends with it.
fn runs_one_command(proc: &ProcInfo) -> bool {
    proc.argv.iter().skip(1).any(|arg| arg == "-c")
}

/// How Claude Code process `claude_pid` in the pane whose program is
/// `pane_pid` can be moved, or why not.
pub fn move_kind(
    table: &ProcTable,
    pane_pid: u32,
    claude_pid: u32,
) -> Result<MoveKind, &'static str> {
    if claude_pid == pane_pid {
        return Ok(MoveKind::Respawn);
    }
    let parent = table.get(claude_pid).and_then(|proc| proc.ppid);
    if parent != Some(pane_pid) {
        return Err(NESTED);
    }
    match table.get(pane_pid) {
        // `sh -c 'claude …'` where the shell did not hand itself over.
        Some(pane) if is_shell(pane) && runs_one_command(pane) => Ok(MoveKind::Respawn),
        Some(pane) if is_shell(pane) => Ok(MoveKind::Shell),
        _ => Err(NESTED),
    }
}

/// The process runs without permission prompts.
pub fn skips_permissions(argv: &[String]) -> bool {
    argv.iter().enumerate().any(|(at, arg)| {
        arg == "--dangerously-skip-permissions"
            || arg == "--permission-mode=bypassPermissions"
            || (arg == "--permission-mode"
                && argv.get(at + 1).map(String::as_str) == Some("bypassPermissions"))
    })
}

/// A pane that runs Claude Code or carries a mark, as the last collection
/// saw it.
#[derive(Debug, Clone)]
pub struct PaneState {
    pub pane_id: String,
    /// The pane's program.
    pub pane_pid: u32,
    /// Claude Code's process, when it runs.
    pub claude_pid: Option<u32>,
    pub record: Option<ClaudeRecord>,
    pub marks: Marks,
    pub info: PaneClaude,
    /// The state the collector derived (screen and records).
    pub agent_state: Option<crate::model::AgentState>,
    /// The pane's current directory.
    pub cwd: String,
    /// How Claude Code would be moved, when it runs and can be.
    pub kind: Option<MoveKind>,
}

impl PaneState {
    /// Claude Code is between turns: its own status is exactly `idle`
    /// (`shell` means a background command still runs), or, without a
    /// status, its screen says so (D7).
    pub fn idle(&self) -> bool {
        match self.record.as_ref().and_then(|r| r.status.as_deref()) {
            Some(status) => status == "idle",
            None => self.agent_state == Some(crate::model::AgentState::Idle),
        }
    }
}

/// Claude Code running in a pane.
#[derive(Debug, Clone)]
pub struct ClaudeProc<'a> {
    pub pid: u32,
    pub record: Option<&'a ClaudeRecord>,
}

/// The pane's `claude` field: present when Claude Code runs in it or it
/// carries a mark.
pub fn pane_claude(
    accounts: &Accounts,
    registry: &Registry,
    table: &ProcTable,
    pane_pid: u32,
    marks: &Marks,
    claude: Option<ClaudeProc<'_>>,
) -> Option<PaneClaude> {
    let Some(claude) = claude else {
        if marks.is_empty() {
            return None;
        }
        return Some(PaneClaude {
            account: None,
            config_dir: None,
            follow: marks.follow(),
            pending: marks.pending.clone(),
            skip_permissions: marks.skip,
            session_id: None,
            movable: false,
            not_movable_reason: Some(NOT_RUNNING.to_string()),
        });
    };
    let record = claude.record;
    let config_dir = record.map(|r| r.config_dir.as_path());
    let account = config_dir.and_then(|dir| accounts.account_of_dir(registry, dir));
    let session_id = record.and_then(|r| r.session_id.clone());
    let argv = table
        .get(claude.pid)
        .map(|proc| proc.argv.as_slice())
        .unwrap_or_default();
    let movable = match (&session_id, &account) {
        (None, _) => Err(NO_SESSION),
        (_, None) => Err("Claude Code runs on a folder that is no account here."),
        _ => move_kind(table, pane_pid, claude.pid).map(drop),
    };
    Some(PaneClaude {
        account,
        config_dir: config_dir.map(|dir| display(accounts, dir)),
        follow: marks.follow(),
        pending: marks.pending.clone(),
        skip_permissions: marks.skip || skips_permissions(argv),
        session_id,
        movable: movable.is_ok(),
        not_movable_reason: movable.err().map(str::to_string),
    })
}

fn display(accounts: &Accounts, dir: &Path) -> String {
    match accounts.paths() {
        Some(paths) => paths.display(dir),
        None => dir.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, ppid: u32, name: &str, argv: &[&str]) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: Some(ppid),
            name: name.into(),
            argv: argv.iter().map(|a| a.to_string()).collect(),
            cwd: None,
        }
    }

    #[test]
    fn panes_move_by_how_claude_runs_in_them() {
        let table = ProcTable::from_procs([
            // Started by the worker: `env … claude` handed itself over.
            proc(10, 1, "claude", &["claude", "--resume", "x"]),
            // By hand in an interactive shell.
            proc(20, 1, "zsh", &["-zsh"]),
            proc(21, 20, "claude", &["claude"]),
            // A shell that kept `-c`.
            proc(30, 1, "fish", &["fish", "-c", "env X=1 claude"]),
            proc(31, 30, "claude", &["claude"]),
            // Over ssh.
            proc(40, 1, "zsh", &["-zsh"]),
            proc(41, 40, "ssh", &["ssh", "box"]),
            proc(42, 41, "claude", &["claude"]),
            // A wrapper that is no shell.
            proc(50, 1, "node", &["node", "wrap.js"]),
            proc(51, 50, "claude", &["claude"]),
        ]);
        assert_eq!(move_kind(&table, 10, 10), Ok(MoveKind::Respawn));
        assert_eq!(move_kind(&table, 20, 21), Ok(MoveKind::Shell));
        assert_eq!(move_kind(&table, 30, 31), Ok(MoveKind::Respawn));
        assert_eq!(move_kind(&table, 40, 42), Err(NESTED));
        assert_eq!(move_kind(&table, 50, 51), Err(NESTED));
    }

    #[test]
    fn skipping_permissions_shows_in_the_arguments() {
        let argv = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert!(skips_permissions(&argv(&[
            "claude",
            "--dangerously-skip-permissions"
        ])));
        assert!(skips_permissions(&argv(&[
            "claude",
            "--permission-mode",
            "bypassPermissions"
        ])));
        assert!(skips_permissions(&argv(&[
            "claude",
            "--permission-mode=bypassPermissions"
        ])));
        assert!(!skips_permissions(&argv(&[
            "claude",
            "--permission-mode",
            "plan"
        ])));
        assert!(!skips_permissions(&argv(&["claude"])));
    }

    #[test]
    fn marks_say_how_a_pane_follows() {
        assert_eq!(Marks::from_raw("", false, "").follow(), Follow::Manual);
        assert!(Marks::from_raw(" ", false, "").is_empty());
        assert_eq!(
            Marks::from_raw("global", false, "").follow(),
            Follow::Global
        );
        let pinned = Marks::from_raw("personal", true, "work");
        assert_eq!(pinned.follow(), Follow::Pinned);
        assert_eq!(pinned.pending.as_deref(), Some("work"));
    }

    #[test]
    fn a_pane_reads_its_account_from_the_folder() {
        let home = super::super::test_support::Home::new("panes");
        let accounts = home.accounts();
        let registry = accounts.registry();
        let table = ProcTable::from_procs([proc(
            10,
            1,
            "claude",
            &["claude", "--dangerously-skip-permissions"],
        )]);
        let record = ClaudeRecord {
            config_dir: home.paths.claude_home.clone(),
            session_id: Some("s1".into()),
            status: Some("idle".into()),
            cwd: Some("/w".into()),
        };
        let marks = Marks::from_raw("global", false, "");
        let pane = pane_claude(
            &accounts,
            &registry,
            &table,
            10,
            &marks,
            Some(ClaudeProc {
                pid: 10,
                record: Some(&record),
            }),
        )
        .unwrap();
        assert_eq!(pane.account.as_deref(), Some("default"));
        assert_eq!(pane.config_dir.as_deref(), Some("~/.claude"));
        assert_eq!(pane.follow, Follow::Global);
        assert!(pane.skip_permissions && pane.movable);
        // A folder no account owns: shown, never moved.
        let stranger = ClaudeRecord {
            config_dir: "/opt/other".into(),
            ..record.clone()
        };
        let pane = pane_claude(
            &accounts,
            &registry,
            &table,
            10,
            &Marks::default(),
            Some(ClaudeProc {
                pid: 10,
                record: Some(&stranger),
            }),
        )
        .unwrap();
        assert_eq!(pane.account, None);
        assert_eq!(pane.config_dir.as_deref(), Some("/opt/other"));
        assert_eq!(pane.follow, Follow::Manual);
        assert!(!pane.movable);
        assert!(pane_claude(&accounts, &registry, &table, 99, &Marks::default(), None).is_none());
        let dead = pane_claude(&accounts, &registry, &table, 99, &marks, None).unwrap();
        assert!(!dead.movable);
    }
}
