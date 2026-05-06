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
        "skills::".to_string(),
        "prompts::".to_string(),
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
            "engine::",
            "state::",
            "stream::",
            "iii.",
            "iii::",
            "mcp::",
            "a2a::",
            "skills::",
            "prompts::",
        ] {
            assert!(
                prefixes.iter().any(|p| p == expected),
                "missing prefix {expected:?}"
            );
        }
    }
}
