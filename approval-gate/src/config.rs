//! YAML-backed runtime settings for [`WorkerConfig`].

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

fn default_topic() -> String {
    "agent::before_function_call".to_string()
}

fn default_approval_state_scope() -> String {
    "approvals".to_string()
}

fn default_default_timeout_ms() -> u64 {
    300_000
}

fn default_classifier_timeout_ms() -> u64 {
    2000
}

/// Per-function iii intercept rule: optional classifier trigger before pending +
/// optional `__from_approval` injection on post-resolve `iii.trigger`.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct InterceptorRule {
    pub function_id: String,
    #[serde(default)]
    pub classifier: Option<String>,
    #[serde(default = "default_classifier_timeout_ms")]
    pub classifier_timeout_ms: u64,
    #[serde(default)]
    pub inject_approval_marker: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    #[serde(default = "default_topic")]
    pub topic: String,
    #[serde(default = "default_approval_state_scope")]
    pub approval_state_scope: String,
    #[serde(default = "default_default_timeout_ms")]
    pub default_timeout_ms: u64,
    #[serde(default)]
    pub interceptors: Vec<InterceptorRule>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            topic: default_topic(),
            approval_state_scope: default_approval_state_scope(),
            default_timeout_ms: default_default_timeout_ms(),
            interceptors: Vec::new(),
        }
    }
}

/// Load operator config: flat keys, or iii-style `{ config: { … } }`.
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
        assert_eq!(cfg.topic, default_topic());
        assert_eq!(cfg.approval_state_scope, "approvals");
        assert_eq!(cfg.default_timeout_ms, 300_000);
        assert!(cfg.interceptors.is_empty());
    }

    #[test]
    fn interceptors_default_empty() {
        assert!(WorkerConfig::default().interceptors.is_empty());
    }

    #[test]
    fn interceptors_parse_from_nested_config_block() {
        let yaml = r#"
interceptors:
  - function_id: shell::exec
    classifier: shell::classify_argv
    classifier_timeout_ms: 1500
    inject_approval_marker: true
  - function_id: other::fn
    classifier: null
"#;
        let cfg: WorkerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.interceptors.len(), 2);
        assert_eq!(cfg.interceptors[0].function_id, "shell::exec");
        assert_eq!(
            cfg.interceptors[0].classifier.as_deref(),
            Some("shell::classify_argv")
        );
        assert_eq!(cfg.interceptors[0].classifier_timeout_ms, 1500);
        assert!(cfg.interceptors[0].inject_approval_marker);
        assert_eq!(cfg.interceptors[1].function_id, "other::fn");
        assert!(cfg.interceptors[1].classifier.is_none());
        assert!(!cfg.interceptors[1].inject_approval_marker);
    }

    #[test]
    fn interceptor_rule_marker_defaults_false() {
        let yaml = r#"
interceptors:
  - function_id: x::y
    classifier: c::f
"#;
        let cfg: WorkerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.interceptors.len(), 1);
        assert!(!cfg.interceptors[0].inject_approval_marker);
        assert_eq!(cfg.interceptors[0].classifier_timeout_ms, 2000);
    }

    #[test]
    fn nested_config_block_supported() {
        let root: serde_yaml::Value = serde_yaml::from_str(
            "config:\n  topic: agent::hook\n  approval_state_scope: myscope\n  default_timeout_ms: 5000",
        )
        .unwrap();
        let node = root.get("config").cloned().unwrap_or(root);
        let cfg: WorkerConfig = serde_yaml::from_value(node).unwrap();
        assert_eq!(cfg.topic, "agent::hook");
        assert_eq!(cfg.approval_state_scope, "myscope");
        assert_eq!(cfg.default_timeout_ms, 5000);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_empty: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(WorkerConfig::default(), from_empty);
    }
}
