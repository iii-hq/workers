//! Operator-facing runtime configuration.
//!
//! The authoritative value comes from the `configuration` worker at boot
//! (see [`crate::configuration`]); a `--config` YAML file, when passed,
//! only SEEDS the initial registration. Every field has a serde default so
//! an empty object yields a fully-populated config. The console renders the
//! worker-config form from this schema, so every field carries a doc
//! comment.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Root config shape. Unknown keys are rejected so a typo'd field fails
/// loudly instead of silently running the default.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Directory holding one folder per memory bank (`blocks/*.md` +
    /// `facts.jsonl`). A leading `~/` expands to the home directory. An
    /// unwritable directory is boot-fatal — memory must never silently run
    /// in RAM.
    #[serde(default = "default_data_dir")]
    pub data_dir: String,

    /// Bank used when a session selects none (session metadata
    /// `memory_bank` wins, then turn metadata, then this).
    #[serde(default = "default_bank")]
    pub default_bank: String,

    /// Inject the bank's markdown blocks into the system prompt on every
    /// turn (`harness::hook::pre-generate`).
    #[serde(default = "default_true")]
    pub inject_blocks: bool,

    /// Recall facts against the latest user message and append them as one
    /// bounded message on every turn. Appending (rather than mutating the
    /// system prompt) keeps the provider prompt cache warm.
    #[serde(default = "default_true")]
    pub inject_facts: bool,

    /// Maximum facts injected per turn.
    #[serde(default = "default_recall_limit")]
    pub recall_limit: usize,

    /// Token budget (estimated at 4 chars/token) for the injected facts.
    #[serde(default = "default_recall_budget_tokens")]
    pub recall_budget_tokens: u64,

    /// Extract new facts after each completed turn (one `router::complete`
    /// call, off the hot path). Disable to run memory as a purely manual
    /// store.
    #[serde(default = "default_true")]
    pub extraction_enabled: bool,

    /// Model for the extraction call. Empty picks the first model in the
    /// router catalog (logged); pin this in production.
    #[serde(default)]
    pub extraction_model: String,

    /// How many recent user/assistant messages the extraction call reads.
    #[serde(default = "default_extraction_window")]
    pub extraction_window: usize,

    /// Outer budget for one extraction call through `router::complete` (ms).
    #[serde(default = "default_extraction_timeout_ms")]
    pub extraction_timeout_ms: u64,

    /// Cap on facts accepted from a single extraction pass.
    #[serde(default = "default_max_facts_per_turn")]
    pub max_facts_per_turn: usize,

    /// Recency half-life in days for recall ranking. Unpinned facts lose
    /// rank weight as they age; they never lose the data.
    #[serde(default = "default_decay_half_life_days")]
    pub decay_half_life_days: u64,
}

fn default_data_dir() -> String {
    "~/.iii/data/memory".to_string()
}
fn default_bank() -> String {
    "main".to_string()
}
fn default_true() -> bool {
    true
}
fn default_recall_limit() -> usize {
    6
}
fn default_recall_budget_tokens() -> u64 {
    1_200
}
fn default_extraction_window() -> usize {
    12
}
fn default_extraction_timeout_ms() -> u64 {
    60_000
}
fn default_max_facts_per_turn() -> usize {
    8
}
fn default_decay_half_life_days() -> u64 {
    30
}

impl Default for WorkerConfig {
    fn default() -> Self {
        serde_json::from_value(serde_json::json!({})).expect("defaults populate")
    }
}

impl WorkerConfig {
    /// The data directory with a leading `~/` expanded to `$HOME`.
    pub fn resolved_data_dir(&self) -> PathBuf {
        expand_tilde(&self.data_dir)
    }

    /// Structural fields whose change requires a store rebuild (mirrors
    /// context-manager's lease-store reload).
    pub fn boot_signature(&self) -> String {
        self.data_dir.clone()
    }

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

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
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
        assert_eq!(cfg.default_bank, "main");
        assert!(cfg.extraction_enabled);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(WorkerConfig::from_json(&serde_json::json!({ "default_bnak": "x" })).is_err());
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

    #[test]
    fn env_expansion_applies_to_seed_yaml_only() {
        std::env::set_var("MEMORY_TEST_DIR", "/tmp/mem-test");
        let cfg = WorkerConfig::from_yaml("data_dir: ${MEMORY_TEST_DIR}").unwrap();
        assert_eq!(cfg.data_dir, "/tmp/mem-test");
    }
}
