use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct SubagentConfig {
    #[serde(default = "default_default_system_prompt")]
    pub default_system_prompt: String,
    #[serde(default = "default_trigger_timeout_ms")]
    pub trigger_timeout_ms: u64,
    #[serde(default = "default_default_max_subagent_depth")]
    pub default_max_subagent_depth: u64,
}

fn default_default_system_prompt() -> String {
    "You are a focused sub-agent. Answer the parent's subtask concisely and stop.".to_string()
}

fn default_trigger_timeout_ms() -> u64 {
    600_000
}

fn default_default_max_subagent_depth() -> u64 {
    3
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            default_system_prompt: default_default_system_prompt(),
            trigger_timeout_ms: default_trigger_timeout_ms(),
            default_max_subagent_depth: default_default_max_subagent_depth(),
        }
    }
}

pub fn load_config(path: &str) -> Result<SubagentConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: SubagentConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: SubagentConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(
            cfg.default_system_prompt,
            "You are a focused sub-agent. Answer the parent's subtask concisely and stop."
        );
        assert_eq!(cfg.trigger_timeout_ms, 600_000);
        assert_eq!(cfg.default_max_subagent_depth, 3);
    }

    #[test]
    fn custom_yaml_overrides() {
        let yaml = r"
default_system_prompt: 'custom'
trigger_timeout_ms: 1000
default_max_subagent_depth: 5
";
        let cfg: SubagentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.default_system_prompt, "custom");
        assert_eq!(cfg.trigger_timeout_ms, 1000);
        assert_eq!(cfg.default_max_subagent_depth, 5);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let yaml_default: SubagentConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(
            yaml_default.default_system_prompt,
            SubagentConfig::default().default_system_prompt
        );
        assert_eq!(
            yaml_default.trigger_timeout_ms,
            SubagentConfig::default().trigger_timeout_ms
        );
        assert_eq!(
            yaml_default.default_max_subagent_depth,
            SubagentConfig::default().default_max_subagent_depth
        );
    }
}
