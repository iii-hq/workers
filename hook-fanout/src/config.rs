use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WorkerConfig {
    #[serde(default = "default_default_timeout_ms")]
    pub default_timeout_ms: u64,
    #[serde(default = "default_min_timeout_ms")]
    pub min_timeout_ms: u64,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_default_timeout_ms() -> u64 {
    10_000
}

fn default_min_timeout_ms() -> u64 {
    50
}

fn default_poll_interval_ms() -> u64 {
    25
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: default_default_timeout_ms(),
            min_timeout_ms: default_min_timeout_ms(),
            poll_interval_ms: default_poll_interval_ms(),
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
        assert_eq!(cfg.default_timeout_ms, 10_000);
        assert_eq!(cfg.min_timeout_ms, 50);
        assert_eq!(cfg.poll_interval_ms, 25);
    }

    #[test]
    fn custom_yaml_overrides() {
        let cfg: WorkerConfig = serde_yaml::from_str(
            "default_timeout_ms: 3000\nmin_timeout_ms: 100\npoll_interval_ms: 10",
        )
        .unwrap();
        assert_eq!(cfg.default_timeout_ms, 3000);
        assert_eq!(cfg.min_timeout_ms, 100);
        assert_eq!(cfg.poll_interval_ms, 10);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let d = WorkerConfig::default();
        assert_eq!(d.default_timeout_ms, default_default_timeout_ms());
        assert_eq!(d.min_timeout_ms, default_min_timeout_ms());
        assert_eq!(d.poll_interval_ms, default_poll_interval_ms());
    }
}
