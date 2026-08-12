//! Operator-facing runtime configuration.
//!
//! The authoritative value comes from the `configuration` worker at boot
//! (see [`crate::configuration`]); a `--config` YAML file, when passed, only
//! SEEDS the initial registration. Every field has a serde default so an empty
//! object yields a fully-populated config, and every field is a per-call
//! tuning knob read from the live snapshot — nothing here requires a restart.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Root config shape. Unknown keys are rejected so a typo'd field fails loudly
/// instead of silently running the default.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Largest canvas source accepted, in bytes. Guards against an excalidraw
    /// scene or generated mermaid text large enough to bloat the state bus.
    #[serde(default = "default_max_source_bytes")]
    pub max_source_bytes: usize,

    /// Most records `canvas::list` returns in one response.
    #[serde(default = "default_max_list")]
    pub max_list: usize,
}

fn default_max_source_bytes() -> usize {
    2 * 1024 * 1024
}

fn default_max_list() -> usize {
    200
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            max_source_bytes: default_max_source_bytes(),
            max_list: default_max_list(),
        }
    }
}

impl WorkerConfig {
    /// Parse a seed config from YAML, expanding `${NAME}` against the process
    /// env FIRST (the seed file is the only path that needs expansion — values
    /// fetched from `configuration::get` are already env-expanded by the
    /// configuration worker), then deserializing.
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        let parsed: Self =
            serde_yaml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))?;
        parsed.validate()
    }

    /// Reject values that parse but cannot mean anything.
    fn validate(self) -> Result<Self, String> {
        if self.max_source_bytes == 0 {
            return Err("max_source_bytes must be at least 1".to_string());
        }
        if self.max_list == 0 {
            return Err("max_list must be at least 1".to_string());
        }
        Ok(self)
    }

    /// Read and parse a YAML seed file (env-expanded — see [`Self::from_yaml`]).
    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    /// Parse a config from a JSON value already env-expanded by the
    /// configuration worker. Does NOT run [`expand_env`] (double expansion
    /// would be a bug) and tolerates a zero-field object (serde defaults fill
    /// in).
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let parsed: Self =
            serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))?;
        parsed.validate()
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
}

/// Expand `${NAME}` and `${NAME:default}` against the process env. An unset
/// variable with no default expands to the empty string, matching the
/// configuration worker's own expansion.
fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let spec = &after[..end];
                let (name, fallback) = match spec.split_once(':') {
                    Some((n, d)) => (n, Some(d)),
                    None => (spec, None),
                };
                match (std::env::var(name), fallback) {
                    (Ok(v), _) => out.push_str(&v),
                    (Err(_), Some(d)) => out.push_str(d),
                    (Err(_), None) => {
                        tracing::warn!(var = %name, "config references undefined env var")
                    }
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
    fn empty_yaml_yields_defaults() {
        let cfg = WorkerConfig::from_yaml("{}").expect("empty object parses");
        assert_eq!(cfg, WorkerConfig::default());
    }

    #[test]
    fn yaml_overrides_each_field() {
        let cfg = WorkerConfig::from_yaml("max_source_bytes: 1024\nmax_list: 7\n")
            .expect("full object parses");
        assert_eq!(cfg.max_source_bytes, 1024);
        assert_eq!(cfg.max_list, 7);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = WorkerConfig::from_yaml("max_lizt: 10\n").expect_err("typo must fail loudly");
        assert!(
            err.contains("max_lizt"),
            "error should name the field: {err}"
        );
    }

    #[test]
    fn zero_limits_are_rejected() {
        for bad in ["max_source_bytes: 0\n", "max_list: 0\n"] {
            let err = WorkerConfig::from_yaml(bad).expect_err("zero limit");
            assert!(err.contains("at least 1"), "{err}");
        }
        let err = WorkerConfig::from_json(&serde_json::json!({ "max_list": 0 }))
            .expect_err("both parse paths validate");
        assert!(err.contains("at least 1"), "{err}");
    }

    #[test]
    fn json_round_trips() {
        let cfg = WorkerConfig {
            max_list: 123,
            ..WorkerConfig::default()
        };
        let back = WorkerConfig::from_json(&cfg.to_json()).expect("round trip");
        assert_eq!(cfg, back);
    }

    #[test]
    fn schema_carries_defaults_as_example() {
        let schema = WorkerConfig::json_schema();
        assert_eq!(schema["example"], WorkerConfig::default().to_json());
        assert!(schema["properties"]["max_list"]["description"].is_string());
    }

    #[test]
    fn env_expansion_applies_to_the_seed_only() {
        std::env::set_var("CANVAS_TEST_LIST", "99");
        let cfg = WorkerConfig::from_yaml("max_list: ${CANVAS_TEST_LIST}\n").expect("expands");
        assert_eq!(cfg.max_list, 99);
        std::env::remove_var("CANVAS_TEST_LIST");

        let cfg =
            WorkerConfig::from_yaml("max_list: ${CANVAS_UNSET_VAR:42}\n").expect("falls back");
        assert_eq!(cfg.max_list, 42);
    }
}
