//! Operator-facing runtime configuration (Path B: the authoritative value
//! lives in the `configuration` worker; every field hot-reloads). The
//! console renders the worker-config form from this schema, so every field
//! carries a doc comment.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Root config shape. Unknown keys are rejected so a typo'd field fails
/// loudly instead of silently running the default.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Run the scheduled consolidation pass. Off = the worker only serves
    /// manual `memory-consolidate::run` calls.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Hours between scheduled passes. Catch-up-on-boot: when the worker
    /// was down past a due pass, it runs shortly after boot instead of
    /// waiting a full interval (last run is persisted in the state worker).
    #[serde(default = "default_interval_hours")]
    pub interval_hours: u64,

    /// Plan without applying: scheduled passes report what WOULD be
    /// superseded but write nothing. Manual runs can override per call.
    #[serde(default)]
    pub dry_run: bool,

    /// Only consolidate these banks. Empty = every bank.
    #[serde(default)]
    pub banks: Vec<String>,

    /// Hard cap on supersedes applied in one pass (safety valve; the rest
    /// waits for the next pass and the report says so).
    #[serde(default = "default_max_supersedes_per_run")]
    pub max_supersedes_per_run: usize,
}

fn default_true() -> bool {
    true
}
fn default_interval_hours() -> u64 {
    24
}
fn default_max_supersedes_per_run() -> usize {
    200
}

impl Default for WorkerConfig {
    fn default() -> Self {
        serde_json::from_value(serde_json::json!({})).expect("defaults populate")
    }
}

impl WorkerConfig {
    /// Parse a seed config from YAML (env-expanded `${NAME}` first — the
    /// seed file is the only path that needs expansion).
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        serde_yaml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))
    }

    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    /// Parse from a JSON value already env-expanded by the configuration
    /// worker. Tolerates a zero-field object (serde defaults fill in).
    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    /// The JSON Schema registered with the `configuration` worker.
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
                    Err(_) => {
                        out.push_str("${");
                        out.push_str(name);
                        out.push('}');
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
    fn empty_object_yields_defaults() {
        let cfg = WorkerConfig::from_json(&serde_json::json!({})).unwrap();
        assert_eq!(cfg, WorkerConfig::default());
        assert!(cfg.enabled);
        assert_eq!(cfg.interval_hours, 24);
        assert!(!cfg.dry_run);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(WorkerConfig::from_json(&serde_json::json!({ "enbaled": true })).is_err());
    }

    #[test]
    fn schema_has_example_and_descriptions() {
        let schema = WorkerConfig::json_schema();
        assert!(schema.get("example").is_some());
        let props = schema["properties"].as_object().unwrap();
        for (name, prop) in props {
            assert!(
                prop.get("description").is_some(),
                "config field `{name}` is missing a doc comment (the console renders it)"
            );
        }
    }
}
