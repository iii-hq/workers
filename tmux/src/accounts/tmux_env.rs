//! The global account in tmux's global environment (agent-accounts D3):
//! plain `claude` started in a new tmux window gets `CLAUDE_CONFIG_DIR` from
//! there, so the worker keeps it in step with `accounts.json`: the global
//! account's folder, or unset for `default`. Without `accounts.json` the
//! worker leaves the variable alone. Shells already running keep the
//! environment they started with.

use super::{Accounts, DEFAULT};
use crate::tmux_cli::{Tmux, TmuxError};

pub const VAR: &str = "CLAUDE_CONFIG_DIR";

/// The value `show-environment -g CLAUDE_CONFIG_DIR` printed: `None` when
/// unset or marked removed (`-CLAUDE_CONFIG_DIR`).
pub fn parse_shown(output: &str) -> Option<String> {
    let line = output.lines().next()?.trim_end_matches('\r');
    line.strip_prefix(VAR)?
        .strip_prefix('=')
        .map(str::to_string)
}

impl Accounts {
    /// What tmux's global `CLAUDE_CONFIG_DIR` should be: `Some(None)` for
    /// unset, `Some(Some(dir))`, or `None` to leave it alone (no registry).
    pub fn wanted_global_env(&self) -> Option<Option<String>> {
        if !self.managed() {
            return None;
        }
        let global = self.registry().global;
        if global == DEFAULT {
            Some(None)
        } else {
            self.env_dir(&global).map(Some)
        }
    }
}

/// The tmux command that makes the global environment say `wanted`.
pub fn set_args(wanted: Option<&str>) -> Vec<String> {
    match wanted {
        Some(dir) => vec![
            "set-environment".into(),
            "-g".into(),
            VAR.into(),
            dir.into(),
        ],
        None => vec!["set-environment".into(), "-gu".into(), VAR.into()],
    }
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

/// Bring tmux's global environment in step with the registry. Returns
/// whether it changed anything; no tmux server is nothing to do.
pub async fn reconcile(tmux: &Tmux, accounts: &Accounts) -> Result<bool, String> {
    let Some(wanted) = accounts.wanted_global_env() else {
        return Ok(false);
    };
    let current = match tmux.run(["show-environment", "-g", VAR]).await {
        Ok(output) => parse_shown(&output),
        Err(TmuxError::NoServer) => return Ok(false),
        Err(TmuxError::Failed(message)) if message.contains("unknown variable") => None,
        Err(error) => return Err(error.sentence()),
    };
    if current == wanted {
        return Ok(false);
    }
    match tmux.run(set_args(wanted.as_deref())).await {
        Ok(_) => {
            tracing::info!(global = ?wanted, "tmux global CLAUDE_CONFIG_DIR set");
            Ok(true)
        }
        Err(TmuxError::NoServer) => Ok(false),
        Err(error) => Err(error.sentence()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::Home;
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
    fn the_registry_says_what_the_environment_holds() {
        let home = Home::new("env");
        let accounts = home.accounts();
        assert_eq!(accounts.wanted_global_env(), None, "no registry: hands off");
        let id = accounts.add("Personal", "green", Some(vec![])).unwrap();
        assert_eq!(accounts.wanted_global_env(), Some(None));
        accounts
            .update_registry(|registry| {
                registry.global = id.clone();
                Ok(())
            })
            .unwrap();
        let dir = home.paths.account_dir(&id).to_string_lossy().into_owned();
        assert_eq!(accounts.wanted_global_env(), Some(Some(dir.clone())));
        assert_eq!(
            set_args(Some(&dir)),
            vec!["set-environment", "-g", "CLAUDE_CONFIG_DIR", dir.as_str()]
        );
        assert_eq!(
            set_args(None),
            vec!["set-environment", "-gu", "CLAUDE_CONFIG_DIR"]
        );
    }
}
