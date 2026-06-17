//! Operator-facing runtime + deployment configuration.
//!
//! The authoritative value comes from the `configuration` worker at boot
//! (see [`crate::configuration`]); a `--config` YAML file, when passed,
//! only SEEDS the initial registration. Every field has a serde default,
//! so an empty object yields a fully-populated config. Unknown keys are
//! rejected so a typo'd field fails loudly instead of silently running
//! the default.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::PermissionMode;

fn default_hook_functions() -> Vec<String> {
    vec!["*".to_string()]
}

fn default_hook_timeout_ms() -> u64 {
    5_000
}

fn default_on_error() -> String {
    "fail_closed".to_string()
}

/// The `harness::hook::pre-trigger` binding the worker registers for
/// itself at startup. Consumed once at boot (part of the boot signature);
/// a live change requires a restart to re-bind the hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HookBinding {
    /// pre_trigger target globs to consult on; omit-equivalent default
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
    // 6-field cron (engine cron worker, config key "expression"): once
    // daily at midnight.
    "0 0 0 * * *".to_string()
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

fn default_pending_timeout_ms() -> i64 {
    1_800_000
}

/// The worker's single configuration entry — runtime wiring AND deployment
/// approval defaults in one schema-validated value. Split on a live update
/// into:
///
/// - The BOOT SIGNATURE (`hook` + `sweep_expression`): consumed ONCE at
///   startup to bind the `harness::hook::pre-trigger` hook and the cron
///   sweep. A config change that alters either is REFUSED on hot-reload
///   (logged "restart required", the previous snapshot kept).
/// - Every OTHER field is a per-call tuning knob (the `*_timeout_ms`
///   budgets and the approval defaults `default_mode` / `always_allow_seed`
///   / `pending_timeout_ms`). When a freshly-fetched config's boot
///   signature matches, the snapshot is swapped live; handlers read the
///   current snapshot per call via [`Deps::config`](crate::functions::Deps::config).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// The `harness::hook::pre-trigger` binding (restart-required).
    #[serde(default)]
    pub hook: HookBinding,
    /// 6-field cron expression for the expiry sweep (restart-required).
    #[serde(default = "default_sweep_expression")]
    pub sweep_expression: String,

    /// Fail-closed budget for the synchronous `policy::check_permissions`
    /// consult.
    #[serde(default = "default_policy_timeout_ms")]
    pub policy_timeout_ms: u64,
    /// Best-effort `session::get` budget inside the hook (record context
    /// fields are omitted when this is exceeded).
    #[serde(default = "default_session_fetch_timeout_ms")]
    pub session_fetch_timeout_ms: u64,
    /// Budget for `state::*` calls.
    #[serde(default = "default_state_timeout_ms")]
    pub state_timeout_ms: u64,
    /// Budget for `harness::function::resolve` calls.
    #[serde(default = "default_harness_timeout_ms")]
    pub harness_timeout_ms: u64,

    /// Effective permission mode for sessions with no stored approval
    /// settings record.
    #[serde(default)]
    pub default_mode: PermissionMode,
    /// Deployment trust profile for auto mode (function ids / globs);
    /// copied into a session's settings on its first mutation.
    #[serde(default)]
    pub always_allow_seed: Vec<String>,
    /// Hold deadline in milliseconds; drives `expires_at` on pending
    /// records.
    #[serde(default = "default_pending_timeout_ms")]
    pub pending_timeout_ms: i64,
}

impl WorkerConfig {
    /// Parse a seed config from YAML, expanding `${NAME}` against the
    /// process env FIRST (the seed file is the only path that needs
    /// expansion — values fetched from `configuration::get` are already
    /// env-expanded by the configuration worker), then deserializing.
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        serde_yaml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))
    }

    /// Read and parse a YAML seed file (env-expanded — see [`Self::from_yaml`]).
    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    /// Parse a config from a JSON value already env-expanded by the
    /// configuration worker. Does NOT run `expand_env` (double expansion
    /// would be a bug) and tolerates a zero-field object (serde defaults
    /// fill in).
    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    /// The JSON Schema registered with the `configuration` worker. Field
    /// doc-comments become property descriptions; the shipped defaults
    /// are attached as a top-level `example`.
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

    /// The **structural** fields — the two trigger bindings. `hook` backs
    /// the `harness::hook::pre-trigger` binding and `sweep_expression` the
    /// cron sweep; a live change re-binds the affected trigger on the fly
    /// (register the new binding, then unregister the old — see
    /// [`crate::configuration`]), so neither needs a restart. Every other
    /// field is a per-call tuning knob read from the live snapshot.
    pub fn boot_signature(&self) -> BootSignature {
        BootSignature {
            hook: self.hook.clone(),
            sweep_expression: self.sweep_expression.clone(),
        }
    }
}

/// Signature of the structural config fields — the two trigger bindings
/// (see [`WorkerConfig::boot_signature`]). An equal signature means only
/// per-call knobs changed (swap the snapshot); a different signature
/// re-binds the changed trigger(s) live.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BootSignature {
    pub hook: HookBinding,
    pub sweep_expression: String,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            hook: HookBinding::default(),
            sweep_expression: default_sweep_expression(),
            policy_timeout_ms: default_policy_timeout_ms(),
            session_fetch_timeout_ms: default_session_fetch_timeout_ms(),
            state_timeout_ms: default_state_timeout_ms(),
            harness_timeout_ms: default_harness_timeout_ms(),
            default_mode: PermissionMode::default(),
            always_allow_seed: Vec::new(),
            pending_timeout_ms: default_pending_timeout_ms(),
        }
    }
}

/// Expand `${NAME}` occurrences against the process environment. Unknown
/// variables expand to the empty string and emit a tracing warning. Only
/// the `--config` seed path uses this — values from `configuration::get`
/// are already expanded by the worker. An unterminated `${` is a literal.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec_wiring() {
        let cfg = WorkerConfig::default();
        assert_eq!(cfg.hook.functions, vec!["*".to_string()]);
        assert_eq!(cfg.hook.timeout_ms, 5_000);
        assert_eq!(cfg.hook.on_error, "fail_closed");
        assert_eq!(cfg.sweep_expression, "0 0 0 * * *");
        assert_eq!(cfg.policy_timeout_ms, 5_000);
        assert_eq!(cfg.session_fetch_timeout_ms, 1_000);
        assert_eq!(cfg.state_timeout_ms, 5_000);
        assert_eq!(cfg.harness_timeout_ms, 10_000);
        assert_eq!(cfg.default_mode, PermissionMode::Manual);
        assert!(cfg.always_allow_seed.is_empty());
        assert_eq!(cfg.pending_timeout_ms, 1_800_000);
    }

    #[test]
    fn defaults_from_empty_object() {
        let from_yaml: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(from_yaml, WorkerConfig::default());
    }

    #[test]
    fn partial_yaml_fills_defaults() {
        let cfg: WorkerConfig =
            serde_yaml::from_str("hook:\n  functions: [\"shell::*\"]\n").unwrap();
        assert_eq!(cfg.hook.functions, vec!["shell::*".to_string()]);
        assert_eq!(cfg.hook.timeout_ms, 5_000);
        assert_eq!(cfg.default_mode, PermissionMode::Manual);
    }

    #[test]
    fn unknown_root_key_is_rejected() {
        let res: Result<WorkerConfig, _> = serde_yaml::from_str("sweep_scheduel: \"x\"\n");
        assert!(res.is_err());
    }

    #[test]
    fn json_schema_has_every_property_with_descriptions_and_example() {
        let schema = WorkerConfig::json_schema();
        let props = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("schema has a properties object");
        for field in [
            "hook",
            "sweep_expression",
            "policy_timeout_ms",
            "session_fetch_timeout_ms",
            "state_timeout_ms",
            "harness_timeout_ms",
            "default_mode",
            "always_allow_seed",
            "pending_timeout_ms",
        ] {
            assert!(
                props.get(field).is_some(),
                "missing schema property {field}"
            );
        }
        // The shipped defaults are attached as a top-level example.
        assert_eq!(
            schema.get("example"),
            Some(&WorkerConfig::default().to_json())
        );
    }

    #[test]
    fn from_json_round_trips_from_default() {
        let cfg = WorkerConfig::default();
        let back = WorkerConfig::from_json(&cfg.to_json()).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn from_json_tolerates_empty_object() {
        let back = WorkerConfig::from_json(&serde_json::json!({})).unwrap();
        assert_eq!(back, WorkerConfig::default());
    }

    #[test]
    fn from_json_round_trips_custom_values() {
        let json = serde_json::json!({
            "default_mode": "auto",
            "always_allow_seed": ["state::get"],
            "pending_timeout_ms": 60_000,
        });
        let cfg = WorkerConfig::from_json(&json).unwrap();
        assert_eq!(cfg.default_mode, PermissionMode::Auto);
        assert_eq!(cfg.always_allow_seed, vec!["state::get".to_string()]);
        assert_eq!(cfg.pending_timeout_ms, 60_000);
        // Unspecified fields fall back to serde defaults.
        assert_eq!(cfg.state_timeout_ms, 5_000);
    }

    #[test]
    fn from_json_rejects_garbage() {
        let err = WorkerConfig::from_json(&serde_json::json!({ "default_mode": 42 })).unwrap_err();
        assert!(err.contains("json parse"), "got: {err}");
        let err = WorkerConfig::from_json(&serde_json::json!("garbage")).unwrap_err();
        assert!(err.contains("json parse"), "got: {err}");
    }

    #[test]
    fn from_yaml_expands_env_var() {
        std::env::set_var("APPROVAL_GATE_TEST_SWEEP", "*/5 * * * * *");
        let cfg =
            WorkerConfig::from_yaml("sweep_expression: \"${APPROVAL_GATE_TEST_SWEEP}\"\n").unwrap();
        assert_eq!(cfg.sweep_expression, "*/5 * * * * *");
        std::env::remove_var("APPROVAL_GATE_TEST_SWEEP");
    }

    #[test]
    fn boot_signature_equal_when_only_tuning_knobs_differ() {
        let base = WorkerConfig::default();
        let tuned = WorkerConfig {
            default_mode: PermissionMode::Auto,
            pending_timeout_ms: base.pending_timeout_ms + 1,
            policy_timeout_ms: base.policy_timeout_ms + 1,
            ..base.clone()
        };
        assert_eq!(base.boot_signature(), tuned.boot_signature());
    }

    #[test]
    fn boot_signature_differs_on_structural_fields() {
        let base = WorkerConfig::default();
        let rebound = WorkerConfig {
            sweep_expression: "*/30 * * * * *".to_string(),
            ..base.clone()
        };
        let rehooked = WorkerConfig {
            hook: HookBinding {
                functions: vec!["shell::*".to_string()],
                ..HookBinding::default()
            },
            ..base.clone()
        };
        assert_ne!(base.boot_signature(), rebound.boot_signature());
        assert_ne!(base.boot_signature(), rehooked.boot_signature());
    }
}
