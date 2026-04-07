use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct SensorConfig {
    #[serde(default = "default_extensions")]
    pub scan_extensions: Vec<String>,
    #[serde(default = "default_max_file_size_kb")]
    pub max_file_size_kb: u64,
    #[serde(default)]
    pub score_weights: ScoreWeights,
    #[serde(default)]
    pub thresholds: Thresholds,
}

fn default_extensions() -> Vec<String> {
    vec![
        "rs".into(),
        "ts".into(),
        "py".into(),
        "js".into(),
        "go".into(),
    ]
}

fn default_max_file_size_kb() -> u64 {
    512
}

#[derive(Deserialize, Debug, Clone)]
pub struct ScoreWeights {
    #[serde(default = "default_complexity_weight")]
    pub complexity: f64,
    #[serde(default = "default_coupling_weight")]
    pub coupling: f64,
    #[serde(default = "default_cohesion_weight")]
    pub cohesion: f64,
    #[serde(default = "default_size_weight")]
    pub size: f64,
    #[serde(default = "default_duplication_weight")]
    pub duplication: f64,
}

fn default_complexity_weight() -> f64 {
    0.25
}
fn default_coupling_weight() -> f64 {
    0.25
}
fn default_cohesion_weight() -> f64 {
    0.20
}
fn default_size_weight() -> f64 {
    0.15
}
fn default_duplication_weight() -> f64 {
    0.15
}

impl Default for ScoreWeights {
    fn default() -> Self {
        ScoreWeights {
            complexity: default_complexity_weight(),
            coupling: default_coupling_weight(),
            cohesion: default_cohesion_weight(),
            size: default_size_weight(),
            duplication: default_duplication_weight(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct Thresholds {
    #[serde(default = "default_degradation_pct")]
    pub degradation_pct: f64,
    #[serde(default = "default_min_score")]
    pub min_score: f64,
}

fn default_degradation_pct() -> f64 {
    10.0
}

fn default_min_score() -> f64 {
    60.0
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            degradation_pct: default_degradation_pct(),
            min_score: default_min_score(),
        }
    }
}

impl Default for SensorConfig {
    fn default() -> Self {
        SensorConfig {
            scan_extensions: default_extensions(),
            max_file_size_kb: default_max_file_size_kb(),
            score_weights: ScoreWeights::default(),
            thresholds: Thresholds::default(),
        }
    }
}

pub fn load_config(path: &str) -> Result<SensorConfig> {
    let contents = std::fs::read_to_string(path)?;
    let config: SensorConfig = serde_yaml::from_str(&contents)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config: SensorConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(config.scan_extensions.len(), 5);
        assert_eq!(config.max_file_size_kb, 512);
        assert!((config.score_weights.complexity - 0.25).abs() < f64::EPSILON);
        assert!((config.thresholds.degradation_pct - 10.0).abs() < f64::EPSILON);
        assert!((config.thresholds.min_score - 60.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_config_custom() {
        let yaml = r#"
scan_extensions: ["rs", "go"]
max_file_size_kb: 256
score_weights:
  complexity: 0.30
  coupling: 0.20
  cohesion: 0.20
  size: 0.15
  duplication: 0.15
thresholds:
  degradation_pct: 5.0
  min_score: 70.0
"#;
        let config: SensorConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.scan_extensions.len(), 2);
        assert_eq!(config.max_file_size_kb, 256);
        assert!((config.score_weights.complexity - 0.30).abs() < f64::EPSILON);
        assert!((config.thresholds.degradation_pct - 5.0).abs() < f64::EPSILON);
        assert!((config.thresholds.min_score - 70.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_sensor_config_default() {
        let config = SensorConfig::default();
        assert_eq!(config.scan_extensions.len(), 5);
        assert_eq!(config.max_file_size_kb, 512);
    }
}
