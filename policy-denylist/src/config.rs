use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    #[serde(default = "default_topic")]
    pub topic: String,
    #[serde(default = "default_denied_tools_vec")]
    pub denied_tools: Vec<String>,
}

fn default_topic() -> String {
    policy_denylist::DEFAULT_TOPIC.to_string()
}

fn default_denied_tools_vec() -> Vec<String> {
    policy_denylist::default_denied_tools()
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            topic: default_topic(),
            denied_tools: default_denied_tools_vec(),
        }
    }
}

/// Load operator config: flat `{ topic, denied_tools }`, or iii-style `{ config: { ... } }`.
pub fn load_config(path: &str) -> Result<WorkerConfig> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    let root: serde_yaml::Value =
        serde_yaml::from_str(&raw).with_context(|| format!("parse YAML in {path}"))?;
    let node = root.get("config").cloned().unwrap_or(root);
    let cfg: WorkerConfig = serde_yaml::from_value(node)
        .with_context(|| format!("deserialize WorkerConfig from {path}"))?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml_mapping() {
        let cfg: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.topic, policy_denylist::DEFAULT_TOPIC);
        assert_eq!(cfg.denied_tools, policy_denylist::default_denied_tools());
    }

    #[test]
    fn custom_yaml_overrides() {
        let cfg: WorkerConfig = serde_yaml::from_str(
            r"
topic: agent::custom
denied_tools:
  - risky
",
        )
        .unwrap();
        assert_eq!(cfg.topic, "agent::custom");
        assert_eq!(cfg.denied_tools, vec!["risky".to_string()]);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_empty: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(WorkerConfig::default(), from_empty);
    }

    #[test]
    fn deserialize_nested_config_block() {
        let root: serde_yaml::Value = serde_yaml::from_str(
            "config:\n  topic: agent::nested\n  denied_tools:\n    - bash:rm -rf",
        )
        .unwrap();
        let node = root.get("config").cloned().unwrap_or(root);
        let cfg: WorkerConfig = serde_yaml::from_value(node).unwrap();
        assert_eq!(cfg.topic, "agent::nested");
        assert_eq!(cfg.denied_tools, vec!["bash:rm -rf".to_string()]);
    }
}
