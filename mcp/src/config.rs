use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct McpConfig {
    #[serde(default = "default_hidden_prefixes")]
    pub hidden_prefixes: Vec<String>,
    #[serde(default = "default_require_expose")]
    pub require_expose: bool,
    #[serde(default = "default_api_path")]
    pub api_path: String,
    /// Which tools `tools/list` offers: the fixed gateway set (`iii_call`,
    /// `iii_observe`, ...), one tool per function, or both.
    #[serde(default)]
    pub tools: ToolMode,
    /// Browser origins allowed besides loopback (exact `scheme://host[:port]`).
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// An MCP session idle this long is ended and its observers disposed.
    #[serde(default = "default_session_idle_ttl_secs")]
    pub session_idle_ttl_secs: u64,
    /// Per-observer event buffer; the oldest are dropped beyond it.
    #[serde(default = "default_max_buffered_events")]
    pub max_buffered_events: usize,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolMode {
    #[default]
    Gateway,
    Functions,
    Both,
}

impl ToolMode {
    pub fn gateway(self) -> bool {
        matches!(self, ToolMode::Gateway | ToolMode::Both)
    }
    pub fn functions(self) -> bool {
        matches!(self, ToolMode::Functions | ToolMode::Both)
    }
}

fn default_session_idle_ttl_secs() -> u64 {
    1800
}

fn default_max_buffered_events() -> usize {
    500
}

pub fn default_hidden_prefixes() -> Vec<String> {
    vec![
        "engine::".to_string(),
        "state::".to_string(),
        "stream::".to_string(),
        "iii.".to_string(),
        "iii::".to_string(),
        "mcp::".to_string(),
        "a2a::".to_string(),
    ]
}

fn default_require_expose() -> bool {
    false
}

fn default_api_path() -> String {
    "/mcp".to_string()
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            hidden_prefixes: default_hidden_prefixes(),
            require_expose: default_require_expose(),
            api_path: default_api_path(),
            tools: ToolMode::default(),
            allowed_origins: Vec::new(),
            session_idle_ttl_secs: default_session_idle_ttl_secs(),
            max_buffered_events: default_max_buffered_events(),
        }
    }
}

pub fn load_config(path: &str) -> Result<McpConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: McpConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: McpConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.hidden_prefixes, default_hidden_prefixes());
        assert!(!cfg.require_expose);
        assert_eq!(cfg.api_path, "/mcp");
    }

    #[test]
    fn custom_yaml_overrides_each_field() {
        let yaml = r#"
hidden_prefixes:
  - "demo::"
require_expose: true
api_path: "/mcp-rs"
"#;
        let cfg: McpConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.hidden_prefixes, vec!["demo::".to_string()]);
        assert!(cfg.require_expose);
        assert_eq!(cfg.api_path, "/mcp-rs");
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_yaml: McpConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = McpConfig::default();
        assert_eq!(from_default.hidden_prefixes, from_yaml.hidden_prefixes);
        assert_eq!(from_default.require_expose, from_yaml.require_expose);
        assert_eq!(from_default.api_path, from_yaml.api_path);
    }

    #[test]
    fn defaults_match_typescript_hidden_prefixes() {
        let prefixes = default_hidden_prefixes();
        for expected in [
            "engine::", "state::", "stream::", "iii.", "iii::", "mcp::", "a2a::",
        ] {
            assert!(
                prefixes.iter().any(|p| p == expected),
                "missing prefix {expected:?}"
            );
        }
    }

    #[test]
    fn gateway_settings_default_sensibly() {
        let cfg: McpConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.tools, ToolMode::Gateway);
        assert!(cfg.allowed_origins.is_empty());
        assert_eq!(cfg.session_idle_ttl_secs, 1800);
        assert_eq!(cfg.max_buffered_events, 500);
        assert_eq!(McpConfig::default().tools, ToolMode::Gateway);
    }

    #[test]
    fn tool_mode_and_origins_parse() {
        let cfg: McpConfig = serde_yaml::from_str(
            "tools: both\nallowed_origins: [\"https://console.example.com\"]\nsession_idle_ttl_secs: 60\nmax_buffered_events: 10\n",
        )
        .unwrap();
        assert_eq!(cfg.tools, ToolMode::Both);
        assert_eq!(
            cfg.allowed_origins,
            vec!["https://console.example.com".to_string()]
        );
        assert_eq!(cfg.session_idle_ttl_secs, 60);
        assert_eq!(cfg.max_buffered_events, 10);
        let f: McpConfig = serde_yaml::from_str("tools: functions").unwrap();
        assert_eq!(f.tools, ToolMode::Functions);
        assert!(f.tools.functions() && !f.tools.gateway());
        assert!(ToolMode::Both.functions() && ToolMode::Both.gateway());
        assert!(serde_yaml::from_str::<McpConfig>("tools: everything").is_err());
    }

    #[test]
    fn the_defunct_skills_and_prompts_prefixes_are_gone() {
        let p = default_hidden_prefixes();
        assert!(!p.iter().any(|x| x == "skills::" || x == "prompts::"));
        assert!(p.iter().any(|x| x == "mcp::"));
    }
}
