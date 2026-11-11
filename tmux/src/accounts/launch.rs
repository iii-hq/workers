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

use super::{DEFAULT, GLOBAL};

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
