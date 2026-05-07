use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Default Anthropic Messages endpoint.
pub const DEFAULT_API_URL: &str = "https://api.anthropic.com/v1/messages";

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct WorkerConfig {
    #[serde(default = "default_max_tokens")]
    pub default_max_tokens: u32,
    #[serde(default = "default_api_url_str")]
    pub default_api_url: String,
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_api_url_str() -> String {
    DEFAULT_API_URL.to_string()
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            default_max_tokens: default_max_tokens(),
            default_api_url: default_api_url_str(),
        }
    }
}

pub fn load_config(path: &str) -> Result<WorkerConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: WorkerConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.default_max_tokens, 4096);
        assert_eq!(cfg.default_api_url, DEFAULT_API_URL);
    }

    #[test]
    fn custom_yaml_overrides() {
        let cfg: WorkerConfig = serde_yaml::from_str(
            "default_max_tokens: 8192\ndefault_api_url: https://example/v1/messages",
        )
        .unwrap();
        assert_eq!(cfg.default_max_tokens, 8192);
        assert_eq!(cfg.default_api_url, "https://example/v1/messages");
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let y: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(WorkerConfig::default(), y);
    }
}
