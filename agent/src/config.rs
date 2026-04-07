use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct AgentConfig {
    #[serde(default = "default_model")]
    pub anthropic_model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default = "default_session_ttl_hours")]
    #[allow(dead_code)]
    pub session_ttl_hours: u64,
    #[serde(default = "default_cron_session_cleanup")]
    pub cron_session_cleanup: String,
}

fn default_model() -> String {
    "claude-haiku-4-5-20251001".to_string()
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_max_iterations() -> u32 {
    10
}

fn default_session_ttl_hours() -> u64 {
    24
}

fn default_cron_session_cleanup() -> String {
    "0 0 * * * *".to_string()
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            anthropic_model: default_model(),
            max_tokens: default_max_tokens(),
            max_iterations: default_max_iterations(),
            session_ttl_hours: default_session_ttl_hours(),
            cron_session_cleanup: default_cron_session_cleanup(),
        }
    }
}

pub fn load_config(path: &str) -> Result<AgentConfig> {
    let contents = std::fs::read_to_string(path)?;
    let config: AgentConfig = serde_yaml::from_str(&contents)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = AgentConfig::default();
        assert_eq!(config.anthropic_model, "claude-haiku-4-5-20251001");
        assert_eq!(config.max_tokens, 4096);
        assert_eq!(config.max_iterations, 10);
        assert_eq!(config.session_ttl_hours, 24);
    }

    #[test]
    fn test_config_from_yaml() {
        let yaml = r#"
anthropic_model: "claude-sonnet-4-20250514"
max_tokens: 8192
max_iterations: 5
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.anthropic_model, "claude-sonnet-4-20250514");
        assert_eq!(config.max_tokens, 8192);
        assert_eq!(config.max_iterations, 5);
        assert_eq!(config.session_ttl_hours, 24);
    }

    #[test]
    fn test_config_empty_yaml() {
        let config: AgentConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(config.anthropic_model, "claude-haiku-4-5-20251001");
        assert_eq!(config.max_tokens, 4096);
    }
}
