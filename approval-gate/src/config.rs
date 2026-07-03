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

/// Default cap on `approval::filesystem-access-watch` re-asks per held call
/// (spec-pr3-approval-gate.md § New function `approval::filesystem-access-watch`,
/// step 7) — chosen to give a user a few genuine retries (a hint that
/// keeps landing on a different directory) without letting a
/// misbehaving/looping caller re-prompt forever.
pub fn default_grant_reask_limit() -> usize {
    3
}

/// JSON Schema for the `rules` array — string shorthands the console
/// renders as an editable list. Matches what the gate actually evaluates:
/// `parse_rules_from_config` reads each entry as a shorthand.
fn rules_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
    serde_json::from_value(json!({
        "type": "array",
        "description": "Permission rules for the gate hook (first match wins). Each entry is a string or object.\n\nString shorthands: bare id/glob → allow; prefix ! → deny; no match → hold.\n\nStructured objects: { \"function\": \"shell::*\", \"action\": \"allow\" | \"deny\", \"modes\": [\"auto\"] } — optional modes scope the rule to manual, auto, or full (omit for all modes). Use modes: [\"auto\"] on allow rules to seed the auto-mode trust list.\n\nExamples:\n• \"state::get\" — allow reads\n• \"shell::*\" — allow any shell worker call\n• \"!approval::*\" — deny the approval decision plane (shipped default)\n• { \"function\": \"web::fetch\", \"action\": \"allow\", \"modes\": [\"auto\"] } — auto-mode trust",
        "items": {
            "type": "string",
            "description": "Function id or glob. Allow: \"web::fetch\", \"coder::*\". Deny: \"!configuration::*\", \"!router::chat\"."
        },
        "default": default_rules_value(),
    }))
    .expect("rules JSON Schema is valid")
}

/// The worker's single configuration entry — deployment approval defaults.
/// Every field hot-reloads via [`crate::configuration`]; handlers read the
/// live snapshot per call through [`Deps::config`](crate::functions::Deps::config).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Effective permission mode for sessions with no stored approval
    /// settings record.
    #[serde(default)]
    pub default_mode: PermissionMode,
    /// Agent permission rules evaluated inline by the gate (first match
    /// wins; no match → the call is held for human approval). String
    /// shorthands: bare id/glob → allow, `!`-prefixed → deny. Structured
    /// objects may set `"modes": ["auto"]` on allow rules to seed the
    /// per-session auto-mode trust list.
    #[serde(default = "default_rules")]
    #[schemars(schema_with = "rules_schema")]
    pub rules: Vec<Value>,
    /// Cap on how many times `approval::filesystem-access-watch` will re-ask (hold a
    /// call again) for jail-scope `filesystem_access_request` rejections on the SAME
    /// function call before standing down and letting the error reach the
    /// model. Default 3.
    #[serde(default = "default_grant_reask_limit")]
    pub grant_reask_limit: usize,
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

    /// Rule specs parsed from the configured `rules` array.
    pub fn rule_specs(&self) -> Vec<RuleSpec> {
        parse_rules_from_config(&Value::Array(self.rules.clone()))
    }

    /// Auto-mode trust globs derived from allow rules scoped to `auto`.
    pub fn auto_allow_seed(&self) -> Vec<String> {
        crate::permissions::auto_allow_seed_from_specs(&self.rule_specs())
    }

    /// The JSON Schema registered with the `configuration` worker.
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
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            default_mode: PermissionMode::default(),
            rules: default_rules(),
            grant_reask_limit: default_grant_reask_limit(),
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
        assert_eq!(cfg.default_mode, PermissionMode::Manual);
        assert!(!cfg.rules.is_empty());
        assert!(cfg.auto_allow_seed().is_empty());
        assert_eq!(cfg.grant_reask_limit, 3);
    }

    #[test]
    fn partial_yaml_fills_grant_reask_limit_default() {
        let cfg: WorkerConfig = serde_yaml::from_str("default_mode: auto\n").unwrap();
        assert_eq!(cfg.grant_reask_limit, 3);
    }

    #[test]
    fn grant_reask_limit_is_configurable() {
        let cfg: WorkerConfig = serde_yaml::from_str("grant_reask_limit: 5\n").unwrap();
        assert_eq!(cfg.grant_reask_limit, 5);
    }

    #[test]
    fn defaults_from_empty_object() {
        let from_yaml: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(from_yaml, WorkerConfig::default());
    }

    #[test]
    fn partial_yaml_fills_defaults() {
        let cfg: WorkerConfig = serde_yaml::from_str("default_mode: auto\n").unwrap();
        assert_eq!(cfg.default_mode, PermissionMode::Auto);
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
        for field in ["default_mode", "rules", "grant_reask_limit"] {
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
    fn from_json_rejects_removed_fields() {
        let err = WorkerConfig::from_json(&serde_json::json!({
            "hook": { "functions": ["*"] }
        }))
        .unwrap_err();
        assert!(err.contains("json parse"), "got: {err}");
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
    fn from_yaml_expands_env_var_in_rules() {
        std::env::set_var("APPROVAL_GATE_TEST_RULE", "state::get");
        let cfg = WorkerConfig::from_yaml("rules:\n  - \"${APPROVAL_GATE_TEST_RULE}\"\n").unwrap();
        assert_eq!(cfg.rules, vec![json!("state::get")]);
        std::env::remove_var("APPROVAL_GATE_TEST_RULE");
    }
}
