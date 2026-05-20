//! Runtime configuration for the `console` worker.
//!
//! The YAML config file exposes one operator-facing knob:
//!
//! - `http_port` — TCP port the worker binds for `/`, `/assets/*`, and
//!   `/ws`. Defaults to `3113`.
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
}

fn default_http_port() -> u16 {
    3113
}

impl Default for ConsoleConfig {
    fn default() -> Self {
        Self {
            http_port: default_http_port(),
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
