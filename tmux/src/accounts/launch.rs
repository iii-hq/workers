//! The commands that start Claude Code on an account (agent-accounts G5,
//! D16), as tmux hands them to the pane's shell. The apps preview the same
//! text, so the forms are fixed:
//!
//! - global, and pinned to the account that is global now: on the slot,
//!   `claude` (the worker runs `env -u CLAUDE_CONFIG_DIR claude` when the
//!   session's or tmux's environment sets `CLAUDE_CONFIG_DIR`);
//! - pinned to another account: `env CLAUDE_CONFIG_DIR=<abs folder> claude`;
//!
//! then ` --dangerously-skip-permissions` when skipping and
//! ` --resume <id>` on a move. A move to the slot is always
//! `env -u CLAUDE_CONFIG_DIR claude …`: a shell keeps the environment it
//! started with.

use super::slot::Place;
use super::{Accounts, GLOBAL};

/// The pane option naming a pane's account: `global` or an id (D4).
pub const MARK_ACCOUNT: &str = "@iii_claude_account";
/// The pane option saying Claude Code skips permission prompts (D16).
pub const MARK_SKIP: &str = "@iii_skip_permissions";
/// The pane option naming the account a pane moves to after its turn (D7).
pub const MARK_PENDING: &str = "@iii_claude_pending";

pub const ROOT_SKIP: &str = "Claude Code refuses to skip permission prompts when it runs as root, as it would on this computer.";

/// `text` as one POSIX shell word: as it is when it holds only characters
/// no shell treats specially, else in single quotes.
pub fn shell_quote(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./:@%+,-=".contains(c))
        && !text.starts_with('=');
    if plain {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

/// How Claude Code is told where to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Plain `claude`: the slot, as the environment has it.
    Plain,
    /// The slot, whatever the environment says: `CLAUDE_CONFIG_DIR` unset.
    Slot,
    /// An account's folder, absolute.
    Dir(String),
}

impl Target {
    /// The words before `claude`, if any.
    fn prefix(&self) -> Option<String> {
        match self {
            Self::Plain => None,
            Self::Slot => Some("env -u CLAUDE_CONFIG_DIR".to_string()),
            Self::Dir(dir) => Some(format!("env CLAUDE_CONFIG_DIR={}", shell_quote(dir))),
        }
    }
}

/// The command that starts Claude Code: on `target`, skipping permission
/// prompts or not, resuming conversation `resume` or starting a new one.
pub fn claude_command(target: &Target, skip_permissions: bool, resume: Option<&str>) -> String {
    let mut words: Vec<String> = target.prefix().into_iter().collect();
    words.push("claude".to_string());
    if skip_permissions {
        words.push("--dangerously-skip-permissions".to_string());
    }
    if let Some(id) = resume {
        words.push("--resume".to_string());
        words.push(shell_quote(id));
    }
    words.join(" ")
}

/// The command that signs in on `target` (`claude auth login`): the slot
/// for the global account (G10), else the account's folder.
pub fn login_command(target: &Target) -> String {
    let target = match target {
        Target::Plain => &Target::Slot,
        other => other,
    };
    let mut words: Vec<String> = target.prefix().into_iter().collect();
    words.extend(["claude", "auth", "login"].map(String::from));
    words.join(" ")
}

/// A planned start of Claude Code: the command and the pane's marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// What the pane runs (G5); the `runs` the create functions return.
    pub command: String,
    /// `global` or the account id, for [`MARK_ACCOUNT`].
    pub account: String,
    pub skip_permissions: bool,
    /// It runs on the slot (plain `claude`).
    pub on_slot: bool,
    pub resume: Option<String>,
}

impl Launch {
    /// The pane options to set: the account mark, and the skip mark when
    /// skipping.
    pub fn marks(&self) -> Vec<(&'static str, &str)> {
        let mut marks = vec![(MARK_ACCOUNT, self.account.as_str())];
        if self.skip_permissions {
            marks.push((MARK_SKIP, "1"));
        }
        marks
    }

    /// The same start with `CLAUDE_CONFIG_DIR` unset outright: what a pane
    /// on the slot runs when its environment sets the variable.
    pub fn explicit(&self) -> String {
        if self.on_slot {
            claude_command(&Target::Slot, self.skip_permissions, self.resume.as_deref())
        } else {
            self.command.clone()
        }
    }
}

impl Accounts {
    /// How to start Claude Code on `account` (`global` or an id).
    pub fn plan_launch(
        &self,
        account: &str,
        skip_permissions: bool,
        resume: Option<&str>,
    ) -> Result<Launch, String> {
        self.plan_launch_as(account, skip_permissions, resume, super::runs_as_root())
    }

    fn plan_launch_as(
        &self,
        account: &str,
        skip_permissions: bool,
        resume: Option<&str>,
        root: bool,
    ) -> Result<Launch, String> {
        let account = account.trim();
        let registry = self.registry();
        if account != GLOBAL && !registry.has(account) {
            return Err(super::manage::no_account(account));
        }
        if skip_permissions && root {
            return Err(ROOT_SKIP.to_string());
        }
        let on_slot = account == GLOBAL || account == registry.global;
        let target = if on_slot {
            Target::Plain
        } else {
            self.folder_target(account)?
        };
        Ok(Launch {
            command: claude_command(&target, skip_permissions, resume),
            account: account.to_string(),
            skip_permissions,
            on_slot,
            resume: resume.map(str::to_string),
        })
    }

    fn folder_target(&self, id: &str) -> Result<Target, String> {
        match self.env_dir(id) {
            Some(dir) if dir.starts_with('/') => Ok(Target::Dir(dir)),
            _ => Err(format!("The account \"{id}\" has no folder.")),
        }
    }

    /// The command that starts Claude Code again on `place` in a move: the
    /// slot with `CLAUDE_CONFIG_DIR` unset outright, or a folder.
    pub fn restart_command(
        &self,
        place: &Place,
        skip_permissions: bool,
        resume: Option<&str>,
    ) -> Result<String, String> {
        if skip_permissions && super::runs_as_root() {
            return Err(ROOT_SKIP.to_string());
        }
        let target = match place {
            Place::Slot => Target::Slot,
            Place::Folder(id) => self.folder_target(id)?,
        };
        Ok(claude_command(&target, skip_permissions, resume))
    }

    /// The target `claude auth login` signs in on for account `id`.
    pub fn login_target(&self, id: &str) -> Result<Target, String> {
        if self.registry().global == id {
            Ok(Target::Slot)
        } else {
            self.folder_target(id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_quoted_only_when_needed() {
        assert_eq!(
            shell_quote("/home/dev/.local/share/iii/claude-accounts/personal"),
            "/home/dev/.local/share/iii/claude-accounts/personal"
        );
        assert_eq!(
            shell_quote("/Users/Me/My Accounts/x"),
            "'/Users/Me/My Accounts/x'"
        );
        assert_eq!(shell_quote("/tmp/it's"), r"'/tmp/it'\''s'");
        assert_eq!(shell_quote("/a/$HOME/`x`"), "'/a/$HOME/`x`'");
        assert_eq!(shell_quote("~/x"), "'~/x'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a;b"), "'a;b'");
    }

    #[test]
    fn launch_commands_have_the_three_forms() {
        assert_eq!(claude_command(&Target::Plain, false, None), "claude");
        assert_eq!(
            claude_command(&Target::Slot, false, None),
            "env -u CLAUDE_CONFIG_DIR claude"
        );
        let personal = Target::Dir("/home/dev/.local/share/iii/claude-accounts/personal".into());
        assert_eq!(
            claude_command(&personal, true, None),
            "env CLAUDE_CONFIG_DIR=/home/dev/.local/share/iii/claude-accounts/personal claude --dangerously-skip-permissions"
        );
        assert_eq!(
            claude_command(
                &Target::Plain,
                true,
                Some("06342612-f016-4fc4-b1cc-3aa3fa41bfe8")
            ),
            "claude --dangerously-skip-permissions --resume 06342612-f016-4fc4-b1cc-3aa3fa41bfe8"
        );
        assert_eq!(
            claude_command(&Target::Dir("/a b".into()), false, Some("x")),
            "env CLAUDE_CONFIG_DIR='/a b' claude --resume x"
        );
    }

    #[test]
    fn launches_follow_g5() {
        let home = super::super::test_support::Home::new("launch");
        let accounts = home.accounts();
        let id = accounts.add("Personal", "green", Some(vec![])).unwrap();
        let dir = home.paths.account_dir(&id);
        let launch = accounts.plan_launch_as(&id, true, None, false).unwrap();
        assert_eq!(
            launch.command,
            format!(
                "env CLAUDE_CONFIG_DIR={} claude --dangerously-skip-permissions",
                dir.display()
            )
        );
        assert_eq!(
            launch.marks(),
            vec![(MARK_ACCOUNT, "personal"), (MARK_SKIP, "1")]
        );
        let global = accounts
            .plan_launch_as("global", false, None, true)
            .unwrap();
        assert_eq!(global.command, "claude");
        assert_eq!(global.explicit(), "env -u CLAUDE_CONFIG_DIR claude");
        assert_eq!(global.marks(), vec![(MARK_ACCOUNT, "global")]);
        // Pinned to the account that is global now: on the slot.
        let default = accounts
            .plan_launch_as("default", false, None, false)
            .unwrap();
        assert_eq!(default.command, "claude");
        assert_eq!(default.marks(), vec![(MARK_ACCOUNT, "default")]);
        accounts
            .update_registry(|registry| {
                registry.global = id.clone();
                Ok(())
            })
            .unwrap();
        assert_eq!(
            accounts
                .plan_launch_as("default", false, None, false)
                .unwrap()
                .command,
            format!(
                "env CLAUDE_CONFIG_DIR={} claude",
                home.paths.account_dir("default").display()
            )
        );
        assert_eq!(
            accounts
                .plan_launch_as(&id, false, None, false)
                .unwrap()
                .command,
            "claude"
        );
        assert_eq!(
            accounts.plan_launch_as("global", true, None, true),
            Err(ROOT_SKIP.to_string())
        );
        assert!(accounts
            .plan_launch_as("nobody", false, None, false)
            .is_err());
        assert_eq!(
            accounts
                .restart_command(&Place::Slot, false, Some("s1"))
                .unwrap(),
            "env -u CLAUDE_CONFIG_DIR claude --resume s1"
        );
    }

    #[test]
    fn login_signs_in_on_the_slot_or_the_folder() {
        assert_eq!(
            login_command(&Target::Slot),
            "env -u CLAUDE_CONFIG_DIR claude auth login"
        );
        assert_eq!(
            login_command(&Target::Plain),
            "env -u CLAUDE_CONFIG_DIR claude auth login"
        );
        assert_eq!(
            login_command(&Target::Dir("/r/side project".into())),
            "env CLAUDE_CONFIG_DIR='/r/side project' claude auth login"
        );
    }
}
