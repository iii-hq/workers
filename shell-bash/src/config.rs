use anyhow::Result;
use serde::Deserialize;

use shell_bash::exec::{ExecConfig, DEFAULT_TIMEOUT_MS, MAX_OUTPUT_BYTES, TRIGGER_TIMEOUT_MS};

#[derive(Deserialize, Debug, Clone)]
pub struct WorkerConfig {
    #[serde(default = "default_default_timeout_ms")]
    pub default_timeout_ms: u64,
    #[serde(default = "default_trigger_timeout_ms")]
    pub trigger_timeout_ms: u64,
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: usize,
}

fn default_default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn default_trigger_timeout_ms() -> u64 {
    TRIGGER_TIMEOUT_MS
}

fn default_max_output_bytes() -> usize {
    MAX_OUTPUT_BYTES
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: default_default_timeout_ms(),
            trigger_timeout_ms: default_trigger_timeout_ms(),
            max_output_bytes: default_max_output_bytes(),
        }
    }
}

impl WorkerConfig {
    pub fn exec_config(&self) -> ExecConfig {
        ExecConfig {
            default_timeout_ms: self.default_timeout_ms,
            trigger_timeout_ms: self.trigger_timeout_ms,
            max_output_bytes: self.max_output_bytes,
        }
    }

    pub fn apply_env_overrides(mut self) -> Self {
        if let Ok(v) = std::env::var("SHELL_BASH_DEFAULT_TIMEOUT_MS") {
            if let Ok(ms) = v.trim().parse() {
                self.default_timeout_ms = ms;
            }
        }
        if let Ok(v) = std::env::var("SHELL_BASH_TRIGGER_TIMEOUT_MS") {
            if let Ok(ms) = v.trim().parse() {
                self.trigger_timeout_ms = ms;
            }
        }
        if let Ok(v) = std::env::var("SHELL_BASH_MAX_OUTPUT_BYTES") {
            if let Ok(n) = v.trim().parse() {
                self.max_output_bytes = n;
            }
        }
        self
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
        assert_eq!(cfg.default_timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(cfg.trigger_timeout_ms, TRIGGER_TIMEOUT_MS);
        assert_eq!(cfg.max_output_bytes, MAX_OUTPUT_BYTES);
    }

    #[test]
    fn custom_yaml_overrides() {
        let cfg: WorkerConfig = serde_yaml::from_str(
            r"
default_timeout_ms: 1
trigger_timeout_ms: 2
max_output_bytes: 3
",
        )
        .unwrap();
        assert_eq!(cfg.default_timeout_ms, 1);
        assert_eq!(cfg.trigger_timeout_ms, 2);
        assert_eq!(cfg.max_output_bytes, 3);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let d = WorkerConfig::default();
        let y: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(d.default_timeout_ms, y.default_timeout_ms);
        assert_eq!(d.trigger_timeout_ms, y.trigger_timeout_ms);
        assert_eq!(d.max_output_bytes, y.max_output_bytes);
    }
}
