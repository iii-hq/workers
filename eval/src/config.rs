use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct EvalConfig {
    #[serde(default = "default_retention_hours")]
    pub retention_hours: u64,
    #[serde(default = "default_drift_threshold")]
    pub drift_threshold: f64,
    #[serde(default = "default_cron_drift_check")]
    pub cron_drift_check: String,
    #[serde(default = "default_max_spans_per_function")]
    pub max_spans_per_function: usize,
    #[allow(dead_code)]
    #[serde(default = "default_baseline_window_minutes")]
    pub baseline_window_minutes: u64,
}

fn default_retention_hours() -> u64 {
    24
}

fn default_drift_threshold() -> f64 {
    0.15
}

fn default_cron_drift_check() -> String {
    "0 */10 * * * *".to_string()
}

fn default_max_spans_per_function() -> usize {
    1000
}

fn default_baseline_window_minutes() -> u64 {
    60
}

impl Default for EvalConfig {
    fn default() -> Self {
        EvalConfig {
            retention_hours: default_retention_hours(),
            drift_threshold: default_drift_threshold(),
            cron_drift_check: default_cron_drift_check(),
            max_spans_per_function: default_max_spans_per_function(),
            baseline_window_minutes: default_baseline_window_minutes(),
        }
    }
}

pub fn load_config(path: &str) -> Result<EvalConfig> {
    let contents = std::fs::read_to_string(path)?;
    let config: EvalConfig = serde_yaml::from_str(&contents)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config: EvalConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(config.retention_hours, 24);
        assert!((config.drift_threshold - 0.15).abs() < f64::EPSILON);
        assert_eq!(config.cron_drift_check, "0 */10 * * * *");
        assert_eq!(config.max_spans_per_function, 1000);
        assert_eq!(config.baseline_window_minutes, 60);
    }

    #[test]
    fn test_config_custom() {
        let yaml = r#"
retention_hours: 48
drift_threshold: 0.25
cron_drift_check: "0 */5 * * * *"
max_spans_per_function: 500
baseline_window_minutes: 120
"#;
        let config: EvalConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.retention_hours, 48);
        assert!((config.drift_threshold - 0.25).abs() < f64::EPSILON);
        assert_eq!(config.cron_drift_check, "0 */5 * * * *");
        assert_eq!(config.max_spans_per_function, 500);
        assert_eq!(config.baseline_window_minutes, 120);
    }

    #[test]
    fn test_eval_config_default() {
        let config = EvalConfig::default();
        assert_eq!(config.retention_hours, 24);
        assert!((config.drift_threshold - 0.15).abs() < f64::EPSILON);
        assert_eq!(config.max_spans_per_function, 1000);
    }
}
