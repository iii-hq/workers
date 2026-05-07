use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct SessionTreeConfig {
    #[serde(default = "default_store_backend")]
    pub store_backend: String,
}

fn default_store_backend() -> String {
    "iii_state".to_string()
}

impl Default for SessionTreeConfig {
    fn default() -> Self {
        Self {
            store_backend: default_store_backend(),
        }
    }
}

pub fn load_config(path: &str) -> Result<SessionTreeConfig> {
    let contents = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    let cfg: SessionTreeConfig =
        serde_yaml::from_str(&contents).with_context(|| format!("parse yaml {path}"))?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: SessionTreeConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.store_backend, "iii_state");
    }

    #[test]
    fn custom_yaml_overrides() {
        let cfg: SessionTreeConfig = serde_yaml::from_str("store_backend: memory").unwrap();
        assert_eq!(cfg.store_backend, "memory");
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        assert_eq!(SessionTreeConfig::default().store_backend, "iii_state");
    }
}
