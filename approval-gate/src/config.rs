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
use serde_json::{json, Value};

use crate::permissions::{default_rule_specs, parse_rules_from_config, Permissions, RuleSpec};
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

fn default_session_fetch_timeout_ms() -> u64 {
    1_000
}

fn default_state_timeout_ms() -> u64 {
    5_000
}

fn default_harness_timeout_ms() -> u64 {
    10_000
}

/// The shipped permission rules as a JSON array of shorthand strings — the
/// serde default for [`WorkerConfig::rules`] and the value seeded into the
/// configuration entry on first boot. Sourced from
/// [`crate::permissions::default_rule_specs`] so the in-memory defaults and
/// the console-editable list never drift.
pub fn default_rules_value() -> Vec<Value> {
    default_rule_specs()
        .into_iter()
        .filter_map(|r| match r {
            RuleSpec::Shorthand(s) => Some(Value::String(s)),
            _ => None,
        })
        .collect()
}

fn default_rules() -> Vec<Value> {
    default_rules_value()
}

/// JSON Schema for the `rules` array — string shorthands the console
/// renders as an editable list. Matches what the gate actually evaluates:
/// `parse_rules_from_config` reads each entry as a shorthand.
fn rules_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
    serde_json::from_value(json!({
        "type": "array",
        "description": "Permission rules for the gate hook (first match wins). Each entry is a string: bare id/glob → allow; prefix ! → deny; no match → hold for human approval.\n\nExamples:\n• \"state::get\" — allow reads\n• \"shell::*\" — allow any shell worker call\n• \"!approval::*\" — deny the approval decision plane (shipped default)\n• \"!state::set\" — deny state writes\n• \"*\" — allow everything not denied above (order matters)",
        "items": {
            "type": "string",
            "description": "Function id or glob. Allow: \"web::fetch\", \"coder::*\". Deny: \"!configuration::*\", \"!router::chat\"."
        },
        "default": default_rules_value(),
    }))
    .expect("rules JSON Schema is valid")
}

/// The worker's single configuration entry — runtime wiring AND deployment
/// approval defaults in one schema-validated value. Split on a live update
/// into:
///
/// - The BOOT SIGNATURE (`hook`): consumed ONCE at startup to bind the
///   `harness::hook::pre-trigger` hook. A config change that alters it is
///   re-bound live (register the new binding, then unregister the old —
///   see [`crate::configuration`]).
/// - Every OTHER field is a per-call tuning knob (the `*_timeout_ms`
///   budgets and the approval defaults `default_mode` / `always_allow_seed`
///   / `rules`). When a freshly-fetched config's boot signature matches,
///   the snapshot is swapped live; handlers read the current snapshot per
///   call via [`Deps::config`](crate::functions::Deps::config).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// The `harness::hook::pre-trigger` binding (restart-required).
    #[serde(default)]
    pub hook: HookBinding,
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
    /// Agent permission rules evaluated inline by the gate (first match
    /// wins; no match → the call is held for human approval). String
    /// shorthands: bare id/glob → allow, `!`-prefixed → deny.
    #[serde(default = "default_rules")]
    #[schemars(schema_with = "rules_schema")]
    pub rules: Vec<Value>,
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

    /// Compile the configured `rules` into the inline permission matcher.
    /// Tolerant: invalid entries are skipped and an empty list yields a
    /// matcher that holds every call (fail-closed, never fail-open).
    pub fn permissions(&self) -> Permissions {
        let specs = parse_rules_from_config(&Value::Array(self.rules.clone()));
        Permissions::compile_tolerant(&specs)
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

    /// The **structural** field — the `harness::hook::pre-trigger` binding.
    /// A live change re-binds the hook on the fly (register the new
    /// binding, then unregister the old — see [`crate::configuration`]), so
    /// it needs no restart. Every other field is a per-call tuning knob
    /// read from the live snapshot.
    pub fn boot_signature(&self) -> BootSignature {
        BootSignature {
            hook: self.hook.clone(),
        }
    }
}

/// Signature of the structural config field — the `harness::hook::pre-trigger`
/// binding (see [`WorkerConfig::boot_signature`]). An equal signature means
/// only per-call knobs changed (swap the snapshot); a different signature
/// re-binds the hook live.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BootSignature {
    pub hook: HookBinding,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            hook: HookBinding::default(),
            session_fetch_timeout_ms: default_session_fetch_timeout_ms(),
            state_timeout_ms: default_state_timeout_ms(),
            harness_timeout_ms: default_harness_timeout_ms(),
            default_mode: PermissionMode::default(),
            always_allow_seed: Vec::new(),
            rules: default_rules(),
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
        assert_eq!(cfg.session_fetch_timeout_ms, 1_000);
        assert_eq!(cfg.state_timeout_ms, 5_000);
        assert_eq!(cfg.harness_timeout_ms, 10_000);
        assert_eq!(cfg.default_mode, PermissionMode::Manual);
        assert!(cfg.always_allow_seed.is_empty());
        assert!(!cfg.rules.is_empty());
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
            "session_fetch_timeout_ms",
            "state_timeout_ms",
            "harness_timeout_ms",
            "default_mode",
            "always_allow_seed",
            "rules",
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
            "rules": ["shell::run", "!state::set"],
        });
        let cfg = WorkerConfig::from_json(&json).unwrap();
        assert_eq!(cfg.default_mode, PermissionMode::Auto);
        assert_eq!(cfg.always_allow_seed, vec!["state::get".to_string()]);
        assert_eq!(cfg.rules, vec![json!("shell::run"), json!("!state::set")]);
        // Unspecified fields fall back to serde defaults.
        assert_eq!(cfg.state_timeout_ms, 5_000);
    }

    #[test]
    fn empty_rules_hold_every_call() {
        let cfg = WorkerConfig {
            rules: Vec::new(),
            ..WorkerConfig::default()
        };
        assert_eq!(cfg.permissions().rule_count(), 0);
    }

    #[test]
    fn default_rules_compile_into_a_matcher() {
        let cfg = WorkerConfig::default();
        assert!(cfg.permissions().rule_count() > 0);
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
        std::env::set_var("APPROVAL_GATE_TEST_SEED", "state::get");
        let cfg =
            WorkerConfig::from_yaml("always_allow_seed: [\"${APPROVAL_GATE_TEST_SEED}\"]\n").unwrap();
        assert_eq!(cfg.always_allow_seed, vec!["state::get".to_string()]);
        std::env::remove_var("APPROVAL_GATE_TEST_SEED");
    }

    #[test]
    fn boot_signature_equal_when_only_tuning_knobs_differ() {
        let base = WorkerConfig::default();
        let tuned = WorkerConfig {
            default_mode: PermissionMode::Auto,
            rules: vec![json!("*")],
            ..base.clone()
        };
        assert_eq!(base.boot_signature(), tuned.boot_signature());
    }

    #[test]
    fn boot_signature_differs_on_the_hook_binding() {
        let base = WorkerConfig::default();
        let rehooked = WorkerConfig {
            hook: HookBinding {
                functions: vec!["shell::*".to_string()],
                ..HookBinding::default()
            },
            ..base.clone()
        };
        assert_ne!(base.boot_signature(), rehooked.boot_signature());
    }
}
