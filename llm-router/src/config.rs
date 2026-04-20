use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    #[serde(default = "default_state_scope")]
    pub state_scope: String,

    #[serde(default = "default_classifier_id")]
    pub classifier_default_id: String,

    #[serde(default = "default_stats_days")]
    pub stats_default_days: u32,

    #[serde(default = "default_health_skip_error_rate")]
    pub health_skip_threshold_error_rate: f64,
}

fn default_state_scope() -> String {
    "llm-router".to_string()
}
fn default_classifier_id() -> String {
    "default".to_string()
}
fn default_stats_days() -> u32 {
    7
}
fn default_health_skip_error_rate() -> f64 {
    0.3
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            state_scope: default_state_scope(),
            classifier_default_id: default_classifier_id(),
            stats_default_days: default_stats_days(),
            health_skip_threshold_error_rate: default_health_skip_error_rate(),
        }
    }
}

pub fn load_config(path: &str) -> Result<RouterConfig> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path))?;
    let cfg: RouterConfig =
        serde_yaml::from_str(&content).with_context(|| format!("parse {}", path))?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let c = RouterConfig::default();
        assert_eq!(c.state_scope, "llm-router");
        assert_eq!(c.classifier_default_id, "default");
        assert_eq!(c.stats_default_days, 7);
    }
}
