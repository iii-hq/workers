use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TurnOrchestratorConfig {
    /// How long (ms) `run::start_and_wait` polls before timing out.
    #[serde(default = "default_sync_default_timeout_ms")]
    pub sync_default_timeout_ms: u64,
    /// How frequently (ms) `run::start_and_wait` checks for a terminal state.
    #[serde(default = "default_sync_poll_interval_ms")]
    pub sync_poll_interval_ms: u64,
    /// URIs to fetch from `iii-directory` at the start of every new chat and
    /// inline into the system prompt after the identity preamble.
    /// Empty list = preamble-only prompt.
    #[serde(default = "default_system_default_skills")]
    pub system_default_skills: Vec<String>,
}

fn default_sync_default_timeout_ms() -> u64 {
    120_000
}

fn default_sync_poll_interval_ms() -> u64 {
    50
}

fn default_system_default_skills() -> Vec<String> {
    vec!["iii://iii".to_string()]
}

impl Default for TurnOrchestratorConfig {
    fn default() -> Self {
        Self {
            sync_default_timeout_ms: default_sync_default_timeout_ms(),
            sync_poll_interval_ms: default_sync_poll_interval_ms(),
            system_default_skills: default_system_default_skills(),
        }
    }
}

pub fn load_config(path: &str) -> Result<TurnOrchestratorConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: TurnOrchestratorConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: TurnOrchestratorConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.sync_default_timeout_ms, 120_000);
        assert_eq!(cfg.sync_poll_interval_ms, 50);
    }

    #[test]
    fn custom_yaml_overrides_each_field() {
        let yaml = "sync_default_timeout_ms: 60000\nsync_poll_interval_ms: 100\n";
        let cfg: TurnOrchestratorConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.sync_default_timeout_ms, 60_000);
        assert_eq!(cfg.sync_poll_interval_ms, 100);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_empty: TurnOrchestratorConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = TurnOrchestratorConfig::default();
        assert_eq!(
            from_empty.sync_default_timeout_ms,
            from_default.sync_default_timeout_ms
        );
        assert_eq!(
            from_empty.sync_poll_interval_ms,
            from_default.sync_poll_interval_ms
        );
    }

    #[test]
    fn missing_file_errors() {
        assert!(load_config("/no/such/path/for/config.yaml").is_err());
    }

    #[test]
    fn system_default_skills_defaults_to_iii_uri() {
        let cfg: TurnOrchestratorConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(
            cfg.system_default_skills,
            vec!["iii://iii".to_string()],
            "default config must pre-load the iii skill at chat start"
        );
    }

    #[test]
    fn system_default_skills_accepts_empty_list() {
        let cfg: TurnOrchestratorConfig =
            serde_yaml::from_str("system_default_skills: []").unwrap();
        assert!(cfg.system_default_skills.is_empty());
    }

    #[test]
    fn system_default_skills_accepts_custom_list() {
        let yaml = "system_default_skills:\n  - iii://iii\n  - iii://shell\n";
        let cfg: TurnOrchestratorConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            cfg.system_default_skills,
            vec!["iii://iii".to_string(), "iii://shell".to_string()]
        );
    }
}
