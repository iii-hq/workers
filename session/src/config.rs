use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SessionConfig {
    #[serde(default = "default_store_backend")]
    pub store_backend: String,
    #[serde(default = "default_engine_url")]
    pub engine_url: String,
    #[serde(default = "default_state_scope")]
    pub state_scope: String,
}

fn default_store_backend() -> String {
    "iii_state".to_string()
}

fn default_engine_url() -> String {
    "ws://127.0.0.1:49134".to_string()
}

fn default_state_scope() -> String {
    "agent".to_string()
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            store_backend: default_store_backend(),
            engine_url: default_engine_url(),
            state_scope: default_state_scope(),
        }
    }
}

pub fn load_config(path: &str) -> Result<SessionConfig> {
    let contents = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    let cfg: SessionConfig =
        serde_yaml::from_str(&contents).with_context(|| format!("parse yaml {path}"))?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: SessionConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.store_backend, "iii_state");
        assert_eq!(cfg.engine_url, "ws://127.0.0.1:49134");
        assert_eq!(cfg.state_scope, "agent");
    }

    #[test]
    fn custom_yaml_overrides() {
        let cfg: SessionConfig = serde_yaml::from_str(
            r#"
store_backend: memory
engine_url: "ws://example:49134"
state_scope: custom
"#,
        )
        .unwrap();
        assert_eq!(cfg.store_backend, "memory");
        assert_eq!(cfg.engine_url, "ws://example:49134");
        assert_eq!(cfg.state_scope, "custom");
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let d = SessionConfig::default();
        assert_eq!(d.store_backend, default_store_backend());
        assert_eq!(d.engine_url, default_engine_url());
        assert_eq!(d.state_scope, default_state_scope());
    }
}
