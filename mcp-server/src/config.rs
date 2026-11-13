//! `config.yaml`: the endpoint path, the Origin allow-list, and session and
//! observe limits. Unknown keys (e.g. `tools`, `hidden_prefixes`,
//! `require_expose` from mcp <= 0.6, `max_buffered_events` from 0.7
//! previews) are ignored.

use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct McpConfig {
    /// HTTP path of the endpoint (POST, GET for the session's stream, DELETE
    /// to end a session). Several mcp-server workers share one http worker on
    /// different paths.
    #[serde(default = "default_api_path")]
    pub api_path: String,
    /// Browser origins allowed besides loopback (exact `scheme://host[:port]`).
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// An MCP session idle this long is ended and its observers disposed.
    #[serde(default = "default_session_idle_ttl_secs")]
    pub session_idle_ttl_secs: u64,
    /// Live MCP sessions; `initialize` is refused (503) beyond it.
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,
    /// Observers one session may have.
    #[serde(default = "default_max_observers_per_session")]
    pub max_observers_per_session: usize,
    /// Observations held per session; the oldest are dropped beyond it.
    #[serde(default = "default_max_buffered_observations")]
    pub max_buffered_observations: usize,
}

impl McpConfig {
    pub fn session_limits(&self) -> crate::sessions::SessionLimits {
        crate::sessions::SessionLimits {
            max_sessions: self.max_sessions,
            max_observers: self.max_observers_per_session,
            max_buffered: self.max_buffered_observations,
            idle_ttl: std::time::Duration::from_secs(self.session_idle_ttl_secs),
        }
    }
}

fn default_api_path() -> String {
    "/mcp".to_string()
}

fn default_session_idle_ttl_secs() -> u64 {
    1800
}

fn default_max_sessions() -> usize {
    16
}

fn default_max_observers_per_session() -> usize {
    100
}

fn default_max_buffered_observations() -> usize {
    500
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            api_path: default_api_path(),
            allowed_origins: Vec::new(),
            session_idle_ttl_secs: default_session_idle_ttl_secs(),
            max_sessions: default_max_sessions(),
            max_observers_per_session: default_max_observers_per_session(),
            max_buffered_observations: default_max_buffered_observations(),
        }
    }
}

pub const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

/// Where to connect: `--url`, then `III_MCP_ENGINE_URL`, then `III_URL`.
/// Compose injects `III_URL` (the engine itself) into every worker, so the
/// proxy is named by the variable compose never sets.
pub fn engine_url(
    flag: Option<String>,
    mcp_env: Option<String>,
    iii_env: Option<String>,
) -> String {
    [flag, mcp_env, iii_env]
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_ENGINE_URL.to_string())
}

pub fn load_config(path: &str) -> Result<McpConfig> {
    let contents = std::fs::read_to_string(path)?;
    Ok(serde_yaml::from_str(&contents)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: McpConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.api_path, "/mcp");
        assert!(cfg.allowed_origins.is_empty());
        assert_eq!(
            (
                cfg.session_idle_ttl_secs,
                cfg.max_sessions,
                cfg.max_observers_per_session,
                cfg.max_buffered_observations
            ),
            (1800, 16, 100, 500)
        );
        let d = McpConfig::default();
        assert_eq!(d.max_buffered_observations, 500);
        assert_eq!((d.max_sessions, d.max_observers_per_session), (16, 100));
    }

    #[test]
    fn the_shipped_config_yaml_is_the_defaults() {
        let cfg: McpConfig = serde_yaml::from_str(include_str!("../config.yaml")).unwrap();
        let d = McpConfig::default();
        assert_eq!(
            (
                cfg.api_path,
                cfg.session_idle_ttl_secs,
                cfg.max_sessions,
                cfg.max_observers_per_session,
                cfg.max_buffered_observations
            ),
            (
                d.api_path,
                d.session_idle_ttl_secs,
                d.max_sessions,
                d.max_observers_per_session,
                d.max_buffered_observations
            )
        );
        assert!(!include_str!("../config.yaml").contains("event"));
    }

    #[test]
    fn settings_parse_into_session_limits() {
        let cfg: McpConfig = serde_yaml::from_str(
            "api_path: /x\nallowed_origins: [\"https://console.example.com\"]\nsession_idle_ttl_secs: 60\nmax_sessions: 4\nmax_observers_per_session: 8\nmax_buffered_observations: 10\n",
        )
        .unwrap();
        assert_eq!(cfg.api_path, "/x");
        assert_eq!(
            cfg.allowed_origins,
            vec!["https://console.example.com".to_string()]
        );
        let l = cfg.session_limits();
        assert_eq!(
            (
                l.max_sessions,
                l.max_observers,
                l.max_buffered,
                l.idle_ttl.as_secs()
            ),
            (4, 8, 10, 60)
        );
    }

    #[test]
    fn keys_from_the_per_function_era_are_ignored() {
        // A config.yaml written for mcp <= 0.6 still loads.
        let cfg: McpConfig = serde_yaml::from_str(
            "tools: functions\nhidden_prefixes: [\"engine::\"]\nrequire_expose: true\nmax_buffered_events: 3\n",
        )
        .unwrap();
        assert_eq!(cfg.api_path, "/mcp");
        // Renamed: max_buffered_observations.
        assert_eq!(cfg.max_buffered_observations, 500);
    }

    #[test]
    fn engine_url_prefers_flag_then_mcp_env_then_iii_url() {
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            engine_url(some("ws://flag"), some("ws://mcp"), some("ws://iii")),
            "ws://flag"
        );
        // Compose injects III_URL (the engine); III_MCP_ENGINE_URL names the proxy.
        assert_eq!(
            engine_url(None, some("ws://mcp"), some("ws://iii")),
            "ws://mcp"
        );
        assert_eq!(engine_url(None, None, some("ws://iii")), "ws://iii");
        assert_eq!(engine_url(None, None, None), DEFAULT_ENGINE_URL);
        // Blank values count as unset.
        assert_eq!(
            engine_url(some(" "), some(""), some("ws://iii")),
            "ws://iii"
        );
    }
}
