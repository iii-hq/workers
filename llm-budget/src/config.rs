use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    #[serde(default = "default_skills_trigger_timeout_ms")]
    pub skills_trigger_timeout_ms: u64,
    #[serde(default = "default_skills_handshake_deadline_secs")]
    pub skills_handshake_deadline_secs: u64,
}

fn default_skills_trigger_timeout_ms() -> u64 {
    5000
}

fn default_skills_handshake_deadline_secs() -> u64 {
    180
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            skills_trigger_timeout_ms: default_skills_trigger_timeout_ms(),
            skills_handshake_deadline_secs: default_skills_handshake_deadline_secs(),
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
        assert_eq!(cfg.skills_trigger_timeout_ms, 5000);
        assert_eq!(cfg.skills_handshake_deadline_secs, 180);
    }

    #[test]
    fn custom_yaml_overrides() {
        let y = "
skills_trigger_timeout_ms: 3000
skills_handshake_deadline_secs: 60
";
        let cfg: WorkerConfig = serde_yaml::from_str(y).unwrap();
        assert_eq!(cfg.skills_trigger_timeout_ms, 3000);
        assert_eq!(cfg.skills_handshake_deadline_secs, 60);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let yaml: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(yaml, WorkerConfig::default());
    }
}
