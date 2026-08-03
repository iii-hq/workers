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
    /// Largest PDF accepted, in bytes. Guards against a path or a base64 blob
    /// large enough to exhaust memory during parsing.
    #[serde(default = "default_max_input_bytes")]
    pub max_input_bytes: u64,

    /// Default cap on the characters of extracted text or markdown returned in
    /// one response. A capped response still reports the true total, so the
    /// caller knows what it did not receive. Per-call `max_chars` overrides
    /// this; `0` means no cap.
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,

    /// Characters of leading content included as a preview alongside a capped
    /// or omitted body.
    #[serde(default = "default_preview_chars")]
    pub preview_chars: usize,

    /// Largest number of positioned text items returned by `pdf::extract-items`
    /// in one response. A dense page carries thousands.
    #[serde(default = "default_max_items")]
    pub max_items: usize,

    /// Pages sampled when classifying a document. Sampling is what keeps
    /// classification at tens of milliseconds on a large file. `0` scans every
    /// page.
    #[serde(default = "default_classify_sample_pages")]
    pub classify_sample_pages: usize,

    /// Text-drawing operators a page needs before it counts as a text page.
    #[serde(default = "default_min_text_ops_per_page")]
    pub min_text_ops_per_page: usize,

    /// Share of sampled pages (0.0 to 1.0) that must look like text pages
    /// before the whole document is called text-based.
    #[serde(default = "default_text_page_ratio_threshold")]
    pub text_page_ratio_threshold: f32,
}

fn default_max_input_bytes() -> u64 {
    256 * 1024 * 1024
}

fn default_max_chars() -> usize {
    40_000
}

fn default_preview_chars() -> usize {
    600
}

fn default_max_items() -> usize {
    5_000
}

fn default_classify_sample_pages() -> usize {
    8
}

fn default_min_text_ops_per_page() -> usize {
    3
}

fn default_text_page_ratio_threshold() -> f32 {
    0.6
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            max_input_bytes: default_max_input_bytes(),
            max_chars: default_max_chars(),
            preview_chars: default_preview_chars(),
            max_items: default_max_items(),
            classify_sample_pages: default_classify_sample_pages(),
            min_text_ops_per_page: default_min_text_ops_per_page(),
            text_page_ratio_threshold: default_text_page_ratio_threshold(),
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
        serde_yaml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))
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

    /// Effective character cap for one response: the per-call override when
    /// present, else the configured default. `0` means uncapped.
    pub fn effective_max_chars(&self, requested: Option<usize>) -> usize {
        requested.unwrap_or(self.max_chars)
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
        let cfg = WorkerConfig::from_yaml(
            "max_input_bytes: 1024\n\
             max_chars: 10\n\
             preview_chars: 5\n\
             max_items: 7\n\
             classify_sample_pages: 3\n\
             min_text_ops_per_page: 9\n\
             text_page_ratio_threshold: 0.25\n",
        )
        .expect("full object parses");
        assert_eq!(cfg.max_input_bytes, 1024);
        assert_eq!(cfg.max_chars, 10);
        assert_eq!(cfg.preview_chars, 5);
        assert_eq!(cfg.max_items, 7);
        assert_eq!(cfg.classify_sample_pages, 3);
        assert_eq!(cfg.min_text_ops_per_page, 9);
        assert!((cfg.text_page_ratio_threshold - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = WorkerConfig::from_yaml("max_charz: 10\n").expect_err("typo must fail loudly");
        assert!(
            err.contains("max_charz"),
            "error should name the field: {err}"
        );
    }

    #[test]
    fn json_round_trips() {
        let cfg = WorkerConfig {
            max_chars: 123,
            ..WorkerConfig::default()
        };
        let back = WorkerConfig::from_json(&cfg.to_json()).expect("round trip");
        assert_eq!(cfg, back);
    }

    #[test]
    fn schema_carries_defaults_as_example() {
        let schema = WorkerConfig::json_schema();
        assert_eq!(schema["example"], WorkerConfig::default().to_json());
        assert!(schema["properties"]["max_chars"]["description"].is_string());
    }

    #[test]
    fn per_call_max_chars_overrides_the_default() {
        let cfg = WorkerConfig::default();
        assert_eq!(cfg.effective_max_chars(None), cfg.max_chars);
        assert_eq!(cfg.effective_max_chars(Some(7)), 7);
        assert_eq!(cfg.effective_max_chars(Some(0)), 0);
    }

    #[test]
    fn env_expansion_applies_to_the_seed_only() {
        std::env::set_var("PDF_TEST_CHARS", "99");
        let cfg = WorkerConfig::from_yaml("max_chars: ${PDF_TEST_CHARS}\n").expect("expands");
        assert_eq!(cfg.max_chars, 99);
        std::env::remove_var("PDF_TEST_CHARS");

        let cfg = WorkerConfig::from_yaml("max_chars: ${PDF_UNSET_VAR:42}\n").expect("falls back");
        assert_eq!(cfg.max_chars, 42);
    }
}
