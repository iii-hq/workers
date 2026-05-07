use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WorkerConfig {
    #[serde(default = "default_engine_url")]
    pub engine_url: String,
}

fn default_engine_url() -> String {
    "ws://127.0.0.1:49134".to_string()
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            engine_url: default_engine_url(),
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
        assert_eq!(cfg.engine_url, "ws://127.0.0.1:49134");
    }

    #[test]
    fn custom_yaml_overrides() {
        let cfg: WorkerConfig =
            serde_yaml::from_str(r#"engine_url: "ws://example:49134""#).unwrap();
        assert_eq!(cfg.engine_url, "ws://example:49134");
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        assert_eq!(WorkerConfig::default().engine_url, default_engine_url());
    }
}
