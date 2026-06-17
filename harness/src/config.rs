//! Operator-facing runtime configuration (Path B — configuration worker).
//!
//! The authoritative value comes from the `configuration` worker at boot
//! (see [`crate::configuration`]); a `--config` YAML file, when passed, only
//! SEEDS the initial registration. Every field has a serde default so an empty
//! object yields a fully-populated config.
//!
//! Hot-reload is the default: every numeric knob is read from the live
//! snapshot per call, and the one STRUCTURAL field — `sweep_expression`, the
//! cron binding for the pending-call sweep — is re-bound live on change
//! (register the new binding, then unregister the old). Nothing requires a
//! restart.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Default per-turn generate-step cap when a send omits `max_turns`.
    #[serde(default = "default_max_turns")]
    pub default_max_turns: u32,

    /// Default wait guard for a parked pending call (sub-agent / hook hold)
    /// before the sweep resolves it with an error. Milliseconds.
    #[serde(default = "default_pending_timeout_ms")]
    pub default_pending_timeout_ms: u64,

    /// Sub-agent depth budget: `harness::spawn` past this is refused.
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,

    /// Fan-out budget: non-terminal children of a turn past this is refused.
    #[serde(default = "default_max_children")]
    pub max_children: u32,

    /// Output-contract validation retries before a best-effort finalise.
    #[serde(default = "default_max_validation_retries")]
    pub max_validation_retries: u32,

    /// TTL for `harness_idem` webhook-dedupe rows. Seconds.
    #[serde(default = "default_idem_ttl_secs")]
    pub idem_ttl_secs: u64,

    /// RPC timeout for `session::*` calls. Milliseconds.
    #[serde(default = "default_session_timeout_ms")]
    pub session_timeout_ms: u64,

    /// RPC timeout for `context::assemble`. Milliseconds.
    #[serde(default = "default_context_timeout_ms")]
    pub context_timeout_ms: u64,

    /// Outer budget for one `router::chat` stream. Milliseconds.
    #[serde(default = "default_router_timeout_ms")]
    pub router_timeout_ms: u64,

    /// RPC timeout for `engine::*` and generic `iii.trigger` dispatch.
    /// Milliseconds.
    #[serde(default = "default_dispatch_timeout_ms")]
    pub dispatch_timeout_ms: u64,

    /// Minimum interval between streamed `session::update-message` writes
    /// (delta coalescing). Milliseconds; 0 writes every delta.
    #[serde(default = "default_stream_coalesce_ms")]
    pub stream_coalesce_ms: u64,

    /// Cron expression (6-field) for the pending-call expiry sweep. The one
    /// structural field: a change re-binds the cron trigger live.
    #[serde(default = "default_sweep_expression")]
    pub sweep_expression: String,
}

impl WorkerConfig {
    /// Parse a seed config from YAML, expanding `${NAME}` against the process
    /// env FIRST (only the seed path needs this — values from
    /// `configuration::get` are already env-expanded), then deserializing.
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        serde_yaml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))
    }

    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    /// Parse a config from a JSON value already env-expanded by the
    /// configuration worker (does NOT re-expand) and tolerant of a zero-field
    /// object (serde defaults fill in).
    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    /// The JSON Schema registered with the `configuration` worker. Field
    /// doc-comments become property descriptions; the shipped defaults are
    /// attached as a top-level `example`.
    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(WorkerConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("WorkerConfig JSON Schema serializes");
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert(
                    "definitions".into(),
                    serde_json::to_value(&root.definitions).expect("definitions serialize"),
                );
            }
            obj.insert("example".into(), WorkerConfig::default().to_json());
        }
        schema
    }

    /// The structural signature: the one field consumed by a live trigger
    /// binding rather than per call. A change to it re-binds the cron sweep.
    pub fn boot_signature(&self) -> BootSignature {
        BootSignature {
            sweep_expression: self.sweep_expression.clone(),
        }
    }
}

/// Signature of the structurally-bound config (see
/// [`WorkerConfig::boot_signature`]). Two configs with an equal signature
/// differ only in per-call tuning knobs that hot-apply via a snapshot swap.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BootSignature {
    pub sweep_expression: String,
}

fn default_max_turns() -> u32 {
    16
}
fn default_pending_timeout_ms() -> u64 {
    1_800_000
}
fn default_max_depth() -> u32 {
    3
}
fn default_max_children() -> u32 {
    5
}
fn default_max_validation_retries() -> u32 {
    2
}
fn default_idem_ttl_secs() -> u64 {
    86_400
}
fn default_session_timeout_ms() -> u64 {
    10_000
}
fn default_context_timeout_ms() -> u64 {
    320_000
}
fn default_router_timeout_ms() -> u64 {
    320_000
}
fn default_dispatch_timeout_ms() -> u64 {
    300_000
}
fn default_stream_coalesce_ms() -> u64 {
    150
}
fn default_sweep_expression() -> String {
    // 6-field cron (engine cron worker, config key "expression"): once
    // daily at midnight.
    "0 0 0 * * *".to_string()
}

fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => tracing::warn!(var = %name, "config references undefined env var"),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            default_max_turns: default_max_turns(),
            default_pending_timeout_ms: default_pending_timeout_ms(),
            max_depth: default_max_depth(),
            max_children: default_max_children(),
            max_validation_retries: default_max_validation_retries(),
            idem_ttl_secs: default_idem_ttl_secs(),
            session_timeout_ms: default_session_timeout_ms(),
            context_timeout_ms: default_context_timeout_ms(),
            router_timeout_ms: default_router_timeout_ms(),
            dispatch_timeout_ms: default_dispatch_timeout_ms(),
            stream_coalesce_ms: default_stream_coalesce_ms(),
            sweep_expression: default_sweep_expression(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_object() {
        let cfg = WorkerConfig::from_json(&serde_json::json!({})).unwrap();
        assert_eq!(cfg, WorkerConfig::default());
        assert_eq!(cfg.default_max_turns, 16);
        assert_eq!(cfg.max_depth, 3);
        assert_eq!(cfg.max_children, 5);
        assert_eq!(cfg.sweep_expression, "0 0 0 * * *");
    }

    #[test]
    fn unknown_root_key_is_rejected() {
        let err = WorkerConfig::from_json(&serde_json::json!({ "max_turnz": 3 })).unwrap_err();
        assert!(err.contains("json parse"), "got: {err}");
    }

    #[test]
    fn from_json_round_trips() {
        let cfg = WorkerConfig {
            default_max_turns: 32,
            ..WorkerConfig::default()
        };
        let back = WorkerConfig::from_json(&cfg.to_json()).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn boot_signature_tracks_sweep_expression_only() {
        let base = WorkerConfig::default();
        let tuned = WorkerConfig {
            default_max_turns: base.default_max_turns + 1,
            ..base.clone()
        };
        assert_eq!(base.boot_signature(), tuned.boot_signature());
        let restructured = WorkerConfig {
            sweep_expression: "0 */5 * * * *".to_string(),
            ..base.clone()
        };
        assert_ne!(base.boot_signature(), restructured.boot_signature());
    }

    #[test]
    fn json_schema_has_properties_and_example() {
        let schema = WorkerConfig::json_schema();
        assert!(schema
            .get("properties")
            .and_then(|p| p.as_object())
            .is_some());
        assert_eq!(
            schema.get("example"),
            Some(&WorkerConfig::default().to_json())
        );
    }

    #[test]
    fn from_yaml_expands_env_var() {
        std::env::set_var("HARNESS_TEST_CRON", "0 0 * * * *");
        let cfg = WorkerConfig::from_yaml("sweep_expression: \"${HARNESS_TEST_CRON}\"\n").unwrap();
        assert_eq!(cfg.sweep_expression, "0 0 * * * *");
        std::env::remove_var("HARNESS_TEST_CRON");
    }
}
