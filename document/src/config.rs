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
    /// Largest document accepted, in bytes. Guards against a path or a base64
    /// blob large enough to exhaust memory during parsing.
    #[serde(default = "default_max_input_bytes")]
    pub max_input_bytes: u64,

    /// Default cap on the characters of markdown returned in one response. A
    /// capped response still reports the true total, so the caller knows what
    /// it did not receive. Per-call `max_chars` overrides this; `0` means no
    /// cap.
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,

    /// Characters of leading content included as a preview alongside a capped
    /// body.
    #[serde(default = "default_preview_chars")]
    pub preview_chars: usize,

    /// Largest number of embedded assets `document::extract-assets` returns in
    /// one response. A slide deck carries one image per slide, and a long one
    /// would otherwise return hundreds.
    #[serde(default = "default_max_assets")]
    pub max_assets: usize,

    /// Largest single asset returned with its bytes, in bytes. A larger asset
    /// is still listed with its type and size — the caller learns it exists
    /// and can decide — but its payload is left out rather than base64'd into
    /// a response nobody can use.
    #[serde(default = "default_max_asset_bytes")]
    pub max_asset_bytes: u64,

    /// Vision model `document::ocr` reads pages with when a call names none.
    /// Unset means every call has to choose, which is the safer default for a
    /// function that spends money per page.
    #[serde(default)]
    pub ocr_model: Option<String>,

    /// Pages `document::ocr` transcribes in one call. The ceiling is a spend
    /// limit, not a technical one: a caller that passes `pages` decides for
    /// itself, and a caller that does not should not accidentally read a
    /// four-hundred-page scan.
    #[serde(default = "default_max_ocr_pages")]
    pub max_ocr_pages: usize,

    /// Budget for one bus call `document::ocr` makes — rendering a page or
    /// reading it. Rendering starts a browser and a vision model on a long page
    /// is slow, so this is generous next to the other limits here.
    #[serde(default = "default_ocr_timeout_ms")]
    pub ocr_timeout_ms: u64,

    /// Milliseconds to let a rendered page paint before it is captured.
    ///
    /// `browser::navigate` returns on the load event, which for a PDF fires
    /// when the viewer has loaded — not when it has drawn the page. Capturing
    /// on that signal alone photographs an empty viewer, and the model dutifully
    /// reports a blank image.
    #[serde(default = "default_render_settle_ms")]
    pub ocr_render_settle_ms: u64,

    /// Cache page transcriptions in the `state` worker, keyed by document
    /// content and page. Re-reading the same scan then costs nothing. Turn it
    /// off for a rig with no `state` worker, or when transcriptions should
    /// never be persisted.
    #[serde(default = "default_true")]
    pub ocr_cache: bool,
}

fn default_max_input_bytes() -> u64 {
    64 * 1024 * 1024
}

fn default_max_chars() -> usize {
    40_000
}

fn default_preview_chars() -> usize {
    600
}

fn default_max_assets() -> usize {
    24
}

fn default_max_asset_bytes() -> u64 {
    8 * 1024 * 1024
}

fn default_max_ocr_pages() -> usize {
    20
}

fn default_ocr_timeout_ms() -> u64 {
    120_000
}

fn default_render_settle_ms() -> u64 {
    2_000
}

fn default_true() -> bool {
    true
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            max_input_bytes: default_max_input_bytes(),
            max_chars: default_max_chars(),
            preview_chars: default_preview_chars(),
            max_assets: default_max_assets(),
            max_asset_bytes: default_max_asset_bytes(),
            ocr_model: None,
            max_ocr_pages: default_max_ocr_pages(),
            ocr_timeout_ms: default_ocr_timeout_ms(),
            ocr_render_settle_ms: default_render_settle_ms(),
            ocr_cache: default_true(),
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
    ///
    /// A zero asset ceiling is the interesting case: `max_assets: 0` would make
    /// `document::extract-assets` always return nothing while reporting
    /// success, which reads as "this deck has no images" rather than as a
    /// misconfiguration.
    fn validate(self) -> Result<Self, String> {
        if self.max_assets == 0 {
            return Err(
                "max_assets must be at least 1; a ceiling of 0 makes every extraction look like \
                 an empty document"
                    .to_string(),
            );
        }
        if self.max_asset_bytes == 0 {
            return Err(
                "max_asset_bytes must be at least 1; a ceiling of 0 drops the bytes of every asset"
                    .to_string(),
            );
        }
        if self.max_ocr_pages == 0 {
            return Err(
                "max_ocr_pages must be at least 1; a ceiling of 0 makes document::ocr transcribe \
                 nothing while reporting success"
                    .to_string(),
            );
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

    /// Effective character cap for one response: the per-call override when
    /// present, else the configured default. `0` means uncapped.
    pub fn effective_max_chars(&self, requested: Option<usize>) -> usize {
        requested.unwrap_or(self.max_chars)
    }

    /// Effective asset ceiling for one response: a per-call request is honoured
    /// only when it asks for FEWER than the configured ceiling. A call cannot
    /// lift an operator's limit.
    pub fn effective_max_assets(&self, requested: Option<usize>) -> usize {
        match requested {
            Some(n) if n > 0 => n.min(self.max_assets),
            _ => self.max_assets,
        }
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
             max_assets: 3\n\
             max_asset_bytes: 2048\n",
        )
        .expect("full object parses");
        assert_eq!(cfg.max_input_bytes, 1024);
        assert_eq!(cfg.max_chars, 10);
        assert_eq!(cfg.preview_chars, 5);
        assert_eq!(cfg.max_assets, 3);
        assert_eq!(cfg.max_asset_bytes, 2048);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = WorkerConfig::from_yaml("max_charz: 10\n").expect_err("typo must fail loudly");
        assert!(
            err.contains("max_charz"),
            "error should name the field: {err}"
        );
    }

    /// A zero ceiling reads as an empty document rather than as a broken
    /// config, so it is refused on both parse paths.
    #[test]
    fn zero_ceilings_are_rejected() {
        let err = WorkerConfig::from_yaml("max_assets: 0\n").expect_err("zero assets");
        assert!(err.contains("max_assets"), "{err}");

        let err = WorkerConfig::from_json(&serde_json::json!({ "max_asset_bytes": 0 }))
            .expect_err("zero asset bytes");
        assert!(err.contains("max_asset_bytes"), "{err}");
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

    /// A per-call asset request narrows the operator's ceiling and never lifts
    /// it: the limit exists to bound one response, and a caller asking for a
    /// thousand images is exactly the case it is there for.
    #[test]
    fn a_call_cannot_raise_the_asset_ceiling() {
        let cfg = WorkerConfig {
            max_assets: 5,
            ..WorkerConfig::default()
        };
        assert_eq!(cfg.effective_max_assets(None), 5);
        assert_eq!(cfg.effective_max_assets(Some(2)), 2);
        assert_eq!(cfg.effective_max_assets(Some(500)), 5);
        assert_eq!(cfg.effective_max_assets(Some(0)), 5);
    }

    #[test]
    fn env_expansion_applies_to_the_seed_only() {
        std::env::set_var("DOCUMENT_TEST_CHARS", "99");
        let cfg = WorkerConfig::from_yaml("max_chars: ${DOCUMENT_TEST_CHARS}\n").expect("expands");
        assert_eq!(cfg.max_chars, 99);
        std::env::remove_var("DOCUMENT_TEST_CHARS");

        let cfg =
            WorkerConfig::from_yaml("max_chars: ${DOCUMENT_UNSET_VAR:42}\n").expect("falls back");
        assert_eq!(cfg.max_chars, 42);
    }
}
