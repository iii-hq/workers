//! The commands that start Claude Code on an account (agent-accounts §3.5,
//! D3, D16), as tmux hands them to the pane's shell. The apps preview the
//! same text, so the forms are fixed:
//!
//! - global: `claude` (tmux's global environment says which folder, D3);
//! - pinned `default`: `env -u CLAUDE_CONFIG_DIR claude`;
//! - pinned other: `env CLAUDE_CONFIG_DIR=<abs dir> claude`;
//!
//! then ` --dangerously-skip-permissions` when skipping and
//! ` --resume <id>` on a move.

use super::{Accounts, DEFAULT, GLOBAL};

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

/// How Claude Code is told which account to run on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Whatever tmux's global environment says (a pane that follows the
    /// global account).
    Global,
    /// The default account: `CLAUDE_CONFIG_DIR` unset (D1).
    Default,
    /// Another account's folder, absolute.
    Dir(String),
}

impl Target {
    /// The target of `account` (`global`, `default` or an id) whose folder
    /// is `dir` (for an id).
    pub fn of(account: &str, dir: Option<&str>) -> Result<Self, String> {
        match (account, dir) {
            (GLOBAL, _) => Ok(Self::Global),
            (DEFAULT, _) => Ok(Self::Default),
            (_, Some(dir)) if dir.starts_with('/') => Ok(Self::Dir(dir.to_string())),
            _ => Err(format!("The account \"{account}\" has no folder.")),
        }
    }

    /// The words before `claude`, if any.
    fn prefix(&self) -> Option<String> {
        match self {
            Self::Global => None,
            Self::Default => Some("env -u CLAUDE_CONFIG_DIR".to_string()),
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

/// The command that signs account `target` in (`claude auth login`). It
/// never follows the global account: it signs in the one asked for.
pub fn login_command(target: &Target) -> String {
    let target = match target {
        Target::Global => &Target::Default,
        other => other,
    };
    let mut words: Vec<String> = target.prefix().into_iter().collect();
    words.extend(["claude", "auth", "login"].map(String::from));
    words.join(" ")
}

/// A planned start of Claude Code: the command and the pane's marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// What the pane runs (§3.5); the `runs` the create functions return.
    pub command: String,
    /// `global` or the account id, for [`MARK_ACCOUNT`].
    pub account: String,
    pub skip_permissions: bool,
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
        if account != GLOBAL && !self.registry().has(account) {
            return Err(super::manage::no_account(account));
        }
        if skip_permissions && root {
            return Err(ROOT_SKIP.to_string());
        }
        let target = Target::of(account, self.env_dir(account).as_deref())?;
        Ok(Launch {
            command: claude_command(&target, skip_permissions, resume),
            account: account.to_string(),
            skip_permissions,
        })
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
        assert_eq!(claude_command(&Target::Global, false, None), "claude");
        assert_eq!(
            claude_command(&Target::Default, false, None),
            "env -u CLAUDE_CONFIG_DIR claude"
        );
        let personal = Target::Dir("/home/dev/.local/share/iii/claude-accounts/personal".into());
        assert_eq!(
            claude_command(&personal, true, None),
            "env CLAUDE_CONFIG_DIR=/home/dev/.local/share/iii/claude-accounts/personal claude --dangerously-skip-permissions"
        );
        assert_eq!(
            claude_command(
                &Target::Global,
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
    fn targets_come_from_accounts() {
        assert_eq!(Target::of("global", None).unwrap(), Target::Global);
        assert_eq!(Target::of("default", Some("/x")).unwrap(), Target::Default);
        assert_eq!(
            Target::of("personal", Some("/r/personal")).unwrap(),
            Target::Dir("/r/personal".into())
        );
        assert!(Target::of("personal", None).is_err());
        assert!(Target::of("personal", Some("relative")).is_err());
    }

    #[test]
    fn launches_check_the_account_and_root() {
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
        assert_eq!(global.marks(), vec![(MARK_ACCOUNT, "global")]);
        assert_eq!(
            accounts
                .plan_launch_as("default", false, None, false)
                .unwrap()
                .command,
            "env -u CLAUDE_CONFIG_DIR claude"
        );
        assert_eq!(
            accounts.plan_launch_as("global", true, None, true),
            Err(ROOT_SKIP.to_string())
        );
        assert!(accounts
            .plan_launch_as("nobody", false, None, false)
            .is_err());
    }

    #[test]
    fn login_signs_in_the_account_asked_for() {
        assert_eq!(
            login_command(&Target::Default),
            "env -u CLAUDE_CONFIG_DIR claude auth login"
        );
        assert_eq!(
            login_command(&Target::Global),
            "env -u CLAUDE_CONFIG_DIR claude auth login"
        );
        assert_eq!(
            login_command(&Target::Dir("/r/side project".into())),
            "env CLAUDE_CONFIG_DIR='/r/side project' claude auth login"
        );
    }
}
