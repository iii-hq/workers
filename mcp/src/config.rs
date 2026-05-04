//! Worker runtime config. Mirrors `skills/src/config.rs`.

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct McpConfig {
    #[serde(default = "default_api_path")]
    pub api_path: String,
    #[serde(default = "default_state_timeout_ms")]
    pub state_timeout_ms: u64,
    #[serde(default = "default_hidden_prefixes")]
    pub hidden_prefixes: Vec<String>,
    /// When true, only functions tagged `metadata.mcp.expose == true` are
    /// surfaced in `tools/list`. Default `false` preserves the historical
    /// "expose everything except hidden prefixes" behavior. Recommended
    /// `true` for agent-facing deployments where the curated `mem::*` /
    /// `brain::*` etc. surface is what the LLM should see.
    #[serde(default)]
    pub require_expose: bool,
}

fn default_api_path() -> String {
    "mcp".to_string()
}

fn default_state_timeout_ms() -> u64 {
    30_000
}

/// Function-id prefixes that are never advertised in `tools/list` and
/// rejected at `tools/call`. Mirrors the hard floor in
/// [`skills/src/functions/skills.rs`](../../skills/src/functions/skills.rs)
/// (`ALWAYS_HIDDEN_PREFIXES`). Operators can extend this in `config.yaml`.
fn default_hidden_prefixes() -> Vec<String> {
    vec![
        "engine::".into(),
        "state::".into(),
        "stream::".into(),
        "iii.".into(),
        "iii::".into(),
        "mcp::".into(),
        "a2a::".into(),
        "skills::".into(),
        "prompts::".into(),
    ]
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            api_path: default_api_path(),
            state_timeout_ms: default_state_timeout_ms(),
            hidden_prefixes: default_hidden_prefixes(),
            require_expose: false,
        }
    }
}

pub fn load_config(path: &str) -> Result<McpConfig> {
    let contents = std::fs::read_to_string(path)?;
    let mut cfg: McpConfig = serde_yaml::from_str(&contents)?;
    // The iii engine prepends its own '/' to api_path during route
    // matching. A YAML value like "/mcp" would register as "//mcp" and
    // 404, so normalize at load time and warn the operator.
    if cfg.api_path.starts_with('/') {
        let normalized = cfg.api_path.trim_start_matches('/').to_string();
        tracing::warn!(
            original = %cfg.api_path,
            normalized = %normalized,
            path = %path,
            "config: stripped leading '/' from api_path"
        );
        cfg.api_path = normalized;
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: McpConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.api_path, "mcp");
        assert_eq!(cfg.state_timeout_ms, 30_000);
        assert!(cfg.hidden_prefixes.iter().any(|p| p == "state::"));
        assert!(cfg.hidden_prefixes.iter().any(|p| p == "skills::"));
        assert!(!cfg.require_expose, "require_expose default must be false");
    }

    #[test]
    fn require_expose_can_be_enabled_via_yaml() {
        let cfg: McpConfig = serde_yaml::from_str("require_expose: true").unwrap();
        assert!(cfg.require_expose);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_empty: McpConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = McpConfig::default();
        assert_eq!(from_empty.api_path, from_default.api_path);
        assert_eq!(from_empty.state_timeout_ms, from_default.state_timeout_ms);
        assert_eq!(from_empty.hidden_prefixes, from_default.hidden_prefixes);
    }

    #[test]
    fn custom_yaml_overrides_each_field() {
        let yaml = "\
api_path: custom-mcp
state_timeout_ms: 5000
hidden_prefixes:
  - 'foo::'
  - 'bar::'
";
        let cfg: McpConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.api_path, "custom-mcp");
        assert_eq!(cfg.state_timeout_ms, 5_000);
        assert_eq!(cfg.hidden_prefixes, vec!["foo::", "bar::"]);
    }

    #[test]
    fn malformed_yaml_errors() {
        let err = load_config("/no/such/path/for/mcp.yaml");
        assert!(err.is_err());
    }

    #[test]
    fn yaml_strips_leading_slash_in_api_path() {
        // Round-trip through a tempfile so we exercise the full
        // load_config path (including the normalization warn).
        let dir = std::env::temp_dir();
        let path = dir.join("mcp_test_leading_slash.yaml");
        std::fs::write(&path, "api_path: /mcp\n").unwrap();
        let cfg = load_config(path.to_str().unwrap()).unwrap();
        assert_eq!(cfg.api_path, "mcp", "leading '/' must be stripped");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn yaml_strips_multiple_leading_slashes_in_api_path() {
        let dir = std::env::temp_dir();
        let path = dir.join("mcp_test_double_slash.yaml");
        std::fs::write(&path, "api_path: //mcp\n").unwrap();
        let cfg = load_config(path.to_str().unwrap()).unwrap();
        assert_eq!(cfg.api_path, "mcp");
        let _ = std::fs::remove_file(path);
    }
}
