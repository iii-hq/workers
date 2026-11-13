//! `CLAUDE_CONFIG_DIR` in tmux's environments. The worker no longer keeps
//! the global account there (agent-accounts G1: D3 is gone): plain `claude`
//! runs on the slot. What is left:
//!
//! - a new pane on the slot runs `env -u CLAUDE_CONFIG_DIR claude` when
//!   its session's or tmux's global environment sets the variable (G5);
//! - once, after an update from 0.3.0 (G11), the folder 0.3.0 put in tmux's
//!   global environment is taken out: the only time the worker touches it.

use std::path::Path;

use crate::tmux_cli::{Tmux, TmuxError};

pub const VAR: &str = "CLAUDE_CONFIG_DIR";

/// The value `show-environment [-g] CLAUDE_CONFIG_DIR` printed: `None` when
/// unset or marked removed (`-CLAUDE_CONFIG_DIR`).
pub fn parse_shown(output: &str) -> Option<String> {
    let line = output.lines().next()?.trim_end_matches('\r');
    line.strip_prefix(VAR)?
        .strip_prefix('=')
        .map(str::to_string)
}

/// The `CLAUDE_CONFIG_DIR` of `pane`'s session when it has one of its own,
/// which tmux gives new panes there instead of the global one:
/// `Some(Some(dir))`, `Some(None)` when the session removes it, `None` when
/// the session has none of its own.
pub async fn session_value(tmux: &Tmux, pane: &str) -> Result<Option<Option<String>>, String> {
    match tmux.run(["show-environment", "-t", pane, VAR]).await {
        Ok(output) => {
            let line = output
                .lines()
                .next()
                .unwrap_or_default()
                .trim_end_matches('\r');
            if line == format!("-{VAR}") {
                Ok(Some(None))
            } else {
                Ok(parse_shown(&output).map(Some))
            }
        }
        Err(TmuxError::Failed(message)) if message.contains("unknown variable") => Ok(None),
        Err(error) => Err(error.sentence()),
    }
}

/// tmux's global `CLAUDE_CONFIG_DIR`, `None` when unset or no server runs.
pub async fn global_value(tmux: &Tmux) -> Result<Option<String>, String> {
    match tmux.run(["show-environment", "-g", VAR]).await {
        Ok(output) => Ok(parse_shown(&output)),
        Err(TmuxError::NoServer) => Ok(None),
        Err(TmuxError::Failed(message)) if message.contains("unknown variable") => Ok(None),
        Err(error) => Err(error.sentence()),
    }
}

/// Whether a new process in `pane`'s session gets `CLAUDE_CONFIG_DIR` from
/// tmux: its session's own value, else the global one.
pub async fn pane_sets_var(tmux: &Tmux, pane: &str) -> Result<bool, String> {
    Ok(match session_value(tmux, pane).await? {
        Some(own) => own.is_some(),
        None => global_value(tmux).await?.is_some(),
    })
}

/// Whether `value` names a folder inside `root` (one 0.3.0 set).
pub fn is_under(value: &str, root: &Path) -> bool {
    let path = Path::new(value);
    path.is_absolute()
        && path
            .components()
            .collect::<std::path::PathBuf>()
            .starts_with(root.components().collect::<std::path::PathBuf>())
}

/// G11: take out of tmux's global environment the account folder 0.3.0 put
/// there (a folder inside the accounts `root`); anything else stays.
/// Returns whether it changed anything.
pub async fn unset_old_global(tmux: &Tmux, root: &Path) -> Result<bool, String> {
    let Some(value) = global_value(tmux).await? else {
        return Ok(false);
    };
    if !is_under(&value, root) {
        return Ok(false);
    }
    match tmux.run(["set-environment", "-gu", VAR]).await {
        Ok(_) => {
            tracing::info!(was = %value, "tmux global CLAUDE_CONFIG_DIR of 0.3.0 removed");
            Ok(true)
        }
        Err(TmuxError::NoServer) => Ok(false),
        Err(error) => Err(error.sentence()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_values_read_back() {
        assert_eq!(
            parse_shown("CLAUDE_CONFIG_DIR=/x y/#{pane_id}\n").as_deref(),
            Some("/x y/#{pane_id}")
        );
        assert_eq!(parse_shown("-CLAUDE_CONFIG_DIR\n"), None);
        assert_eq!(parse_shown(""), None);
        assert_eq!(parse_shown("CLAUDE_CONFIG_DIRX=1"), None);
    }

    #[test]
    fn only_folders_under_the_root_are_0_3_0_s() {
        let root = Path::new("/home/dev/.local/share/iii/claude-accounts");
        assert!(is_under(
            "/home/dev/.local/share/iii/claude-accounts/black-trench",
            root
        ));
        assert!(is_under(
            "/home/dev/.local/share/iii/claude-accounts/./x/",
            root
        ));
        assert!(!is_under("/home/dev/.claude-work", root));
        assert!(!is_under(
            "/home/dev/.local/share/iii/claude-accounts-other/x",
            root
        ));
        assert!(!is_under("relative/claude-accounts/x", root));
    }
}
