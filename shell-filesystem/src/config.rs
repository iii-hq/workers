use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    #[serde(default = "default_max_inline_bytes")]
    pub max_inline_bytes: usize,
}

fn default_max_inline_bytes() -> usize {
    262_144
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            max_inline_bytes: default_max_inline_bytes(),
        }
    }
}

pub fn load_config(path: &str) -> Result<WorkerConfig> {
    let contents = std::fs::read_to_string(path).with_context(|| format!("read config {path}"))?;
    serde_yaml::from_str(&contents).with_context(|| format!("parse config yaml {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.max_inline_bytes, default_max_inline_bytes());
    }

    #[test]
    fn custom_yaml_overrides() {
        let cfg: WorkerConfig = serde_yaml::from_str("max_inline_bytes: 1024\n").unwrap();
        assert_eq!(cfg.max_inline_bytes, 1024);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let yaml: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(WorkerConfig::default(), yaml);
    }
}
