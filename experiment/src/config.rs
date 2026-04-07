use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct ExperimentConfig {
    #[serde(default = "default_budget")]
    pub default_budget: u32,
    #[serde(default = "default_max_budget")]
    pub max_budget: u32,
    #[serde(default = "default_timeout_per_run_ms")]
    pub timeout_per_run_ms: u64,
}

fn default_budget() -> u32 {
    20
}

fn default_max_budget() -> u32 {
    100
}

fn default_timeout_per_run_ms() -> u64 {
    30000
}

impl Default for ExperimentConfig {
    fn default() -> Self {
        ExperimentConfig {
            default_budget: default_budget(),
            max_budget: default_max_budget(),
            timeout_per_run_ms: default_timeout_per_run_ms(),
        }
    }
}

pub fn load_config(path: &str) -> Result<ExperimentConfig> {
    let contents = std::fs::read_to_string(path)?;
    let config: ExperimentConfig = serde_yaml::from_str(&contents)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config: ExperimentConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(config.default_budget, 20);
        assert_eq!(config.max_budget, 100);
        assert_eq!(config.timeout_per_run_ms, 30000);
    }

    #[test]
    fn test_config_custom() {
        let yaml = r#"
default_budget: 50
max_budget: 200
timeout_per_run_ms: 60000
"#;
        let config: ExperimentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.default_budget, 50);
        assert_eq!(config.max_budget, 200);
        assert_eq!(config.timeout_per_run_ms, 60000);
    }

    #[test]
    fn test_experiment_config_default() {
        let config = ExperimentConfig::default();
        assert_eq!(config.default_budget, 20);
        assert_eq!(config.max_budget, 100);
        assert_eq!(config.timeout_per_run_ms, 30000);
    }
}
