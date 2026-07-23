//! Runtime configuration for the `console` worker.
//!
//! The YAML config file exposes two operator-facing knobs:
//!
//! - `http_port` — TCP port the worker binds for `/`, `/assets/*`, and
//!   `/ws`. Defaults to `3113`.
//! - `injectable_ui` — kill switch for runtime-injected worker UI
//!   (`console:script` / `console:style` / `console:assets` trigger types,
//!   the `/ui` + `/vendor` routes, and the SPA loader). Defaults to `true`.
//!
//! The iii engine WebSocket URL is set via the CLI (`--url`); see
//! [`DEFAULT_ENGINE_URL`] for the default.

use anyhow::Result;
use serde::Deserialize;

/// Default iii engine WebSocket URL (CLI `--url` default).
pub const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[derive(Deserialize, Debug, Clone)]
pub struct ConsoleConfig {
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_injectable_ui")]
    pub injectable_ui: bool,
}

fn default_http_port() -> u16 {
    3113
}

fn default_injectable_ui() -> bool {
    true
}

impl Default for ConsoleConfig {
    fn default() -> Self {
        Self {
            http_port: default_http_port(),
            injectable_ui: default_injectable_ui(),
        }
    }
}

pub fn load_config(path: &str) -> Result<ConsoleConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: ConsoleConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: ConsoleConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.http_port, 3113);
        assert!(cfg.injectable_ui);
    }

    #[test]
    fn injectable_ui_kill_switch() {
        let cfg: ConsoleConfig = serde_yaml::from_str("injectable_ui: false\n").unwrap();
        assert!(!cfg.injectable_ui);
    }

    #[test]
    fn custom_yaml_overrides_http_port() {
        let cfg: ConsoleConfig = serde_yaml::from_str("http_port: 9090\n").unwrap();
        assert_eq!(cfg.http_port, 9090);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_empty: ConsoleConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = ConsoleConfig::default();
        assert_eq!(from_empty.http_port, from_default.http_port);
    }

    #[test]
    fn missing_file_errors() {
        let err = load_config("/no/such/path/for/console.yaml");
        assert!(err.is_err());
    }
}
