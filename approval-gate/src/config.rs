//! config.yaml parsing — runtime wiring only. Deployment approval
//! defaults live in the `approval-gate` configuration entry
//! (gate_config.rs), never here.

use anyhow::{Context, Result};
use serde::Deserialize;

fn default_hook_functions() -> Vec<String> {
    vec!["*".to_string()]
}

fn default_hook_timeout_ms() -> u64 {
    5_000
}

fn default_on_error() -> String {
    "fail_closed".to_string()
}

/// The `harness::hook::pre_dispatch` binding the worker registers for
/// itself at startup.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HookBinding {
    /// pre_dispatch target globs to consult on; omit-equivalent default
    /// (`["*"]`) consults on every call.
    #[serde(default = "default_hook_functions")]
    pub functions: Vec<String>,
    #[serde(default = "default_hook_timeout_ms")]
    pub timeout_ms: u64,
    /// `fail_closed` is already the harness `pre_*` default — a crashed
    /// gate must deny, not wave calls through.
    #[serde(default = "default_on_error")]
    pub on_error: String,
}

impl Default for HookBinding {
    fn default() -> Self {
        Self {
            functions: default_hook_functions(),
            timeout_ms: default_hook_timeout_ms(),
            on_error: default_on_error(),
        }
    }
}

fn default_sweep_expression() -> String {
    // 6-field cron (engine cron worker, config key "expression"): every
    // minute at second 0.
    "0 * * * * *".to_string()
}

fn default_policy_timeout_ms() -> u64 {
    5_000
}

fn default_session_fetch_timeout_ms() -> u64 {
    1_000
}

fn default_state_timeout_ms() -> u64 {
    5_000
}

fn default_harness_timeout_ms() -> u64 {
    10_000
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    #[serde(default)]
    pub hook: HookBinding,
    #[serde(default = "default_sweep_expression")]
    pub sweep_expression: String,
    #[serde(default = "default_policy_timeout_ms")]
    pub policy_timeout_ms: u64,
    #[serde(default = "default_session_fetch_timeout_ms")]
    pub session_fetch_timeout_ms: u64,
    #[serde(default = "default_state_timeout_ms")]
    pub state_timeout_ms: u64,
    #[serde(default = "default_harness_timeout_ms")]
    pub harness_timeout_ms: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        serde_yaml::from_str("{}").expect("empty config parses to defaults")
    }
}

/// A missing file falls back to defaults (the caller decides); a file
/// that exists but doesn't parse is an error — a typo'd config must
/// never silently run on defaults.
pub fn load_config(path: &str) -> Result<WorkerConfig> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read config {path}"))?;
    serde_yaml::from_str(&raw).with_context(|| format!("parse config {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec_wiring() {
        let cfg = WorkerConfig::default();
        assert_eq!(cfg.hook.functions, vec!["*".to_string()]);
        assert_eq!(cfg.hook.timeout_ms, 5_000);
        assert_eq!(cfg.hook.on_error, "fail_closed");
        assert_eq!(cfg.sweep_expression, "0 * * * * *");
        assert_eq!(cfg.policy_timeout_ms, 5_000);
        assert_eq!(cfg.session_fetch_timeout_ms, 1_000);
    }

    #[test]
    fn partial_yaml_fills_defaults() {
        let cfg: WorkerConfig =
            serde_yaml::from_str("hook:\n  functions: [\"shell::*\"]\n").unwrap();
        assert_eq!(cfg.hook.functions, vec!["shell::*".to_string()]);
        assert_eq!(cfg.hook.timeout_ms, 5_000);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let res: Result<WorkerConfig, _> = serde_yaml::from_str("sweep_scheduel: \"x\"\n");
        assert!(res.is_err());
    }

    #[test]
    fn repo_config_yaml_parses() {
        let cfg = load_config(concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml")).unwrap();
        assert_eq!(cfg, WorkerConfig::default());
    }
}
