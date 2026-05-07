use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ModelsCatalogConfig {
    #[serde(default = "default_engine_url")]
    pub engine_url: String,

    #[serde(default = "default_state_request_timeout_ms")]
    pub state_request_timeout_ms: u64,

    #[serde(default = "default_skills_register_timeout_ms")]
    pub skills_register_timeout_ms: u64,

    #[serde(default = "default_skills_unregister_timeout_ms")]
    pub skills_unregister_timeout_ms: u64,
}

fn default_engine_url() -> String {
    "ws://127.0.0.1:49134".to_string()
}

fn default_state_request_timeout_ms() -> u64 {
    5_000
}

fn default_skills_register_timeout_ms() -> u64 {
    5_000
}

fn default_skills_unregister_timeout_ms() -> u64 {
    2_000
}

impl Default for ModelsCatalogConfig {
    fn default() -> Self {
        Self {
            engine_url: default_engine_url(),
            state_request_timeout_ms: default_state_request_timeout_ms(),
            skills_register_timeout_ms: default_skills_register_timeout_ms(),
            skills_unregister_timeout_ms: default_skills_unregister_timeout_ms(),
        }
    }
}

pub fn load_config(path: &str) -> Result<ModelsCatalogConfig> {
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("read config file {path}"))?;
    let cfg: ModelsCatalogConfig = serde_yaml::from_str(&contents).context("parse config YAML")?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: ModelsCatalogConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.engine_url, "ws://127.0.0.1:49134");
        assert_eq!(cfg.state_request_timeout_ms, 5_000);
        assert_eq!(cfg.skills_register_timeout_ms, 5_000);
        assert_eq!(cfg.skills_unregister_timeout_ms, 2_000);
    }

    #[test]
    fn custom_yaml_overrides() {
        let yaml = r"
engine_url: ws://example:49134
state_request_timeout_ms: 3000
";
        let cfg: ModelsCatalogConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.engine_url, "ws://example:49134");
        assert_eq!(cfg.state_request_timeout_ms, 3_000);
        assert_eq!(cfg.skills_register_timeout_ms, 5_000);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let a = ModelsCatalogConfig::default();
        let b: ModelsCatalogConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(a.engine_url, b.engine_url);
        assert_eq!(a.state_request_timeout_ms, b.state_request_timeout_ms);
        assert_eq!(a.skills_register_timeout_ms, b.skills_register_timeout_ms);
        assert_eq!(
            a.skills_unregister_timeout_ms,
            b.skills_unregister_timeout_ms
        );
    }
}
