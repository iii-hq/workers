//! YAML-backed runtime settings for [`WorkerConfig`].
//!
//! Post-refactor surface (T12):
//!   - `topic` — hook bus topic the gate subscribes to.
//!   - `approval_state_scope` — iii-state scope for approval records.
//!   - `default_timeout_ms` — Pending-row TTL.
//!   - `rules` — the layered ruleset (default + operator-shipped),
//!     evaluated in order with last-match winning.
//!
//! Deleted in T12: `interceptors`, `sweeper_interval_ms`,
//! `InterceptorRule` (the classifier surface is gone).

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

/// Temporary alias retained while register.rs's classifier-alias warning
/// loop still references the symbol. The struct is structurally unused
/// (no fields populated from config) and will be deleted alongside the
/// warning loop when there are no more callers. Provided here so the
/// crate builds.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct InterceptorRule {
    pub function_id: String,
    #[serde(default)]
    pub classifier: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    #[serde(default = "default_topic")]
    pub topic: String,
    #[serde(default = "default_approval_state_scope")]
    pub approval_state_scope: String,
    #[serde(default = "default_default_timeout_ms")]
    pub default_timeout_ms: u64,
    /// Layered permission ruleset. Allow / Deny / Ask actions. Evaluated
    /// last-match-wins; the YAML's curated defaults ship at the bottom,
    /// operator overrides stack on top. See [`crate::rules`].
    #[serde(default)]
    pub rules: crate::rules::Ruleset,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            topic: default_topic(),
            approval_state_scope: default_approval_state_scope(),
            default_timeout_ms: default_default_timeout_ms(),
            rules: Vec::new(),
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
    use crate::rules::{Action, Rule};

    #[test]
    fn defaults_from_empty_yaml_mapping() {
        let cfg: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.topic, default_topic());
        assert_eq!(cfg.approval_state_scope, "approvals");
        assert_eq!(cfg.default_timeout_ms, 300_000);
        assert!(cfg.rules.is_empty());
    }

    #[test]
    fn rules_parse_from_yaml() {
        let yaml = r#"
rules:
  - { permission: "shell::exec", pattern: "git status*", action: allow }
  - { permission: "shell::exec", pattern: "*",            action: ask }
"#;
        let cfg: WorkerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.rules.len(), 2);
        assert_eq!(cfg.rules[0].permission, "shell::exec");
        assert_eq!(cfg.rules[0].pattern, "git status*");
        assert_eq!(cfg.rules[0].action, Action::Allow);
        assert_eq!(cfg.rules[1].action, Action::Ask);
        let _ = Rule {  // smoke check on the imported type
            permission: "x".into(),
            pattern: "*".into(),
            action: Action::Deny,
        };
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
