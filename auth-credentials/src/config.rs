use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StoreBackend {
    #[default]
    IiiState,
    Memory,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WorkerConfig {
    #[serde(default = "default_engine_url")]
    pub engine_url: String,
    #[serde(default)]
    pub store: StoreBackend,
}

fn default_engine_url() -> String {
    "ws://127.0.0.1:49134".to_string()
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            engine_url: default_engine_url(),
            store: StoreBackend::default(),
        }
    }
}

pub fn load_config(path: &str) -> Result<WorkerConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: WorkerConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

/// Resolve store backend: `AUTH_CREDENTIALS_STORE` wins when set; otherwise `cfg.store`.
pub fn resolve_store_backend(cfg: &WorkerConfig) -> StoreBackend {
    match std::env::var("AUTH_CREDENTIALS_STORE").as_deref() {
        Ok("memory") => StoreBackend::Memory,
        Ok("iii_state") => StoreBackend::IiiState,
        Ok(other) if !other.is_empty() => {
            tracing::warn!(
                %other,
                "unknown AUTH_CREDENTIALS_STORE; valid: memory, iii_state — using config `store`"
            );
            cfg.store
        }
        _ => cfg.store,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.engine_url, "ws://127.0.0.1:49134");
        assert_eq!(cfg.store, StoreBackend::IiiState);
    }

    #[test]
    fn custom_yaml_overrides() {
        let cfg: WorkerConfig = serde_yaml::from_str(
            r#"engine_url: "ws://example:49134"
store: memory
"#,
        )
        .unwrap();
        assert_eq!(cfg.engine_url, "ws://example:49134");
        assert_eq!(cfg.store, StoreBackend::Memory);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let d = WorkerConfig::default();
        let cfg: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(d.engine_url, cfg.engine_url);
        assert_eq!(d.store, cfg.store);
    }
}
