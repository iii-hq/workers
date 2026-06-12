//! Operator-facing config. Every field has a `#[serde(default = "…")]`
//! attribute so an empty YAML object still produces a fully-populated
//! struct; `impl Default` mirrors those functions so the binary can fall
//! back to defaults when `--config` is missing or unparseable (see
//! [`binary-worker.md`](../../docs/sops/binary-worker.md) §5).

use std::path::PathBuf;

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CoderConfig {
    /// Legacy single-root form. Honored as a one-entry `base_paths` list.
    /// Setting BOTH `base_path` and `base_paths` is a startup error
    /// (checked at `PathResolver` construction).
    #[serde(default)]
    pub base_path: Option<PathBuf>,

    /// Root directories the worker operates inside. The FIRST entry is
    /// the primary root: relative wire paths resolve against it. Absolute
    /// wire paths are accepted when they canonicalize inside ANY listed
    /// root. When neither this nor `base_path` is set, the effective
    /// default is `["./", "/tmp"]` (resolved at `PathResolver`
    /// construction).
    #[serde(default)]
    pub base_paths: Vec<PathBuf>,

    /// Glob patterns matched against the path *relative to its containing
    /// root*. Matching files can be listed but not
    /// read/written/deleted/created.
    #[serde(default)]
    pub non_accessible_globs: Vec<String>,

    /// Noise-exclusion globs (matched against the path relative to its
    /// containing root, same convention as `non_accessible_globs`).
    /// `coder::tree` and `coder::search` suppress descent into matching
    /// directories and omit matching files; callers opt out per call with
    /// `use_default_excludes: false`. Unlike `non_accessible_globs` this
    /// only HIDES results — it grants no access protection.
    #[serde(default = "default_default_exclude_globs")]
    pub default_exclude_globs: Vec<String>,

    #[serde(default = "default_max_read_bytes")]
    pub max_read_bytes: u64,

    #[serde(default = "default_max_write_bytes")]
    pub max_write_bytes: u64,

    #[serde(default = "default_tree_default_depth")]
    pub tree_default_depth: u32,

    #[serde(default = "default_tree_per_folder_limit")]
    pub tree_per_folder_limit: u32,

    #[serde(default = "default_list_default_page_size")]
    pub list_default_page_size: u32,

    #[serde(default = "default_list_max_page_size")]
    pub list_max_page_size: u32,

    #[serde(default = "default_search_max_matches")]
    pub search_default_max_matches: u32,

    #[serde(default = "default_search_max_line_bytes")]
    pub search_default_max_line_bytes: u32,

    /// Aggregate budget across a single `paths[]` batch call to
    /// `coder::read-file`, measured in BYTES OF RETURNED CONTENT (after
    /// UTF-8 sanitization — invalid bytes expand to 3-byte U+FFFD
    /// replacements before being counted, so the cap bounds what the
    /// caller actually receives). Entries are collected in request order
    /// until this budget is exhausted; an entry reached with zero budget
    /// remaining gets a per-entry C213. Single-path FULL reads are
    /// budgeted by `max_output_bytes` instead; `max_read_bytes` remains
    /// the per-file IO ceiling in every mode.
    #[serde(default = "default_batch_read_budget_bytes")]
    pub batch_read_budget_bytes: u64,

    /// Context budget for single-path FULL reads in `coder::read-file`
    /// (no `line_from`/`line_to`, `stat: false`), measured in BYTES OF
    /// RETURNED CONTENT after UTF-8 sanitization (numbered prefixes
    /// included) — the same accounting unit as `batch_read_budget_bytes`.
    /// A full read whose converted content would exceed this budget
    /// fails with a C213 that reports the file's size and line count and
    /// names the recovery paths (window, stat probe, or per-call
    /// `max_output_bytes` raise, clamped to `max_read_bytes`). Windowed
    /// reads and batch mode are NOT governed by this key.
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: u64,

    /// Aggregate byte budget for one `coder::search` response, measured
    /// in CONVERTED WIRE BYTES at accumulation time — the bytes of the
    /// strings that will actually be serialized (path + matched text +
    /// context lines for content matches; path for path matches), the
    /// same accounting philosophy as `batch_read_budget_bytes`. Exactness
    /// is not required; monotone bounding is. When the next match would
    /// exceed the budget the search stops accumulating and sets
    /// `truncated: true` — it degrades, it never errors.
    #[serde(default = "default_search_response_budget_bytes")]
    pub search_response_budget_bytes: u64,
}

fn default_default_exclude_globs() -> Vec<String> {
    [
        "**/.git/**",
        "**/node_modules/**",
        "**/target/**",
        "**/dist/**",
        "**/.venv/**",
        "**/__pycache__/**",
    ]
    .map(String::from)
    .to_vec()
}
fn default_max_read_bytes() -> u64 {
    10 * 1024 * 1024
}
fn default_max_write_bytes() -> u64 {
    10 * 1024 * 1024
}
fn default_tree_default_depth() -> u32 {
    4
}
fn default_tree_per_folder_limit() -> u32 {
    50
}
fn default_list_default_page_size() -> u32 {
    100
}
fn default_list_max_page_size() -> u32 {
    1_000
}
fn default_search_max_matches() -> u32 {
    1_000
}
fn default_search_max_line_bytes() -> u32 {
    4_096
}
fn default_batch_read_budget_bytes() -> u64 {
    1_048_576
}
fn default_max_output_bytes() -> u64 {
    131_072
}
fn default_search_response_budget_bytes() -> u64 {
    262_144
}

/// A signature of everything the boot-time security jail (`PathResolver`) and
/// the resolver-compiled noise filter depend on. See
/// [`CoderConfig::jail_signature`]. Two configs with an equal signature differ
/// only in numeric tuning knobs that can be hot-applied; any other difference
/// requires a worker restart because the `PathResolver` is built once at boot
/// and is NEVER rebuilt at runtime — it is the security boundary.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JailSignature {
    /// Legacy single-root form. A change re-roots the jail — restart-required
    /// (`PathResolver` compiles the effective root set from this + `base_paths`).
    pub base_path: Option<PathBuf>,
    /// The root directories the jail confines all access to. A change to the
    /// set (or order — the first entry is the primary root) moves the security
    /// boundary, so it is restart-required: the `PathResolver` canonicalizes
    /// these once at boot and refuses to swap them live.
    pub base_paths: Vec<PathBuf>,
    /// The access-deny globs. These are the read/write/delete protection layer
    /// (e.g. `.env`, `*.pem`), compiled into the `PathResolver` at boot. A
    /// change alters the security posture, so it is restart-required — never
    /// relax the jail on a live process.
    pub non_accessible_globs: Vec<String>,
    /// The resolver-compiled noise filter used by `tree`/`search`. Unlike
    /// `non_accessible_globs` this grants NO access protection — it only hides
    /// results — but it is still compiled into the `PathResolver` at boot, so a
    /// change is restart-required for symmetry with the other compiled globsets.
    pub default_exclude_globs: Vec<String>,
}

impl Default for CoderConfig {
    fn default() -> Self {
        Self {
            base_path: None,
            base_paths: Vec::new(),
            non_accessible_globs: Vec::new(),
            default_exclude_globs: default_default_exclude_globs(),
            max_read_bytes: default_max_read_bytes(),
            max_write_bytes: default_max_write_bytes(),
            tree_default_depth: default_tree_default_depth(),
            tree_per_folder_limit: default_tree_per_folder_limit(),
            list_default_page_size: default_list_default_page_size(),
            list_max_page_size: default_list_max_page_size(),
            search_default_max_matches: default_search_max_matches(),
            search_default_max_line_bytes: default_search_max_line_bytes(),
            batch_read_budget_bytes: default_batch_read_budget_bytes(),
            max_output_bytes: default_max_output_bytes(),
            search_response_budget_bytes: default_search_response_budget_bytes(),
        }
    }
}

impl CoderConfig {
    /// Parse a config from a YAML seed string. Expands `${NAME}` against the
    /// process environment FIRST (the seed file is the only path that needs
    /// expansion — values fetched from `configuration::get` are already
    /// env-expanded by the configuration worker), then deserializes. Used only
    /// by the `--config` seed path ([`from_file`]); never on the live fetch.
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        let cfg: CoderConfig =
            serde_yaml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))?;
        Ok(cfg)
    }

    /// Read and parse a YAML seed file (env-expanded — see [`from_yaml`]).
    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    /// Parse a config from a JSON value already env-expanded by the
    /// configuration worker. Does NOT run `expand_env` — double-expansion would
    /// be a bug — and tolerates a zero-field object (serde defaults fill in).
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let cfg: CoderConfig =
            serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))?;
        Ok(cfg)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("CoderConfig serializes")
    }

    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(CoderConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("CoderConfig JSON Schema serializes");
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert(
                    "definitions".into(),
                    serde_json::to_value(&root.definitions).expect("definitions serialize"),
                );
            }
            // Top-level example mirrors the shipped defaults so operators see a
            // ready-to-edit multi-root config (same shape as database's schema).
            obj.insert("example".into(), CoderConfig::default().to_json());
        }
        schema
    }

    /// Build the restart-required jail signature. These four fields are
    /// EVERYTHING the `PathResolver` compiles: the root set (`base_path` +
    /// `base_paths`) that bounds the security jail, the access-deny globs
    /// (`non_accessible_globs`), and the resolver-compiled noise filter
    /// (`default_exclude_globs`). A live config update that changes ANY of them
    /// is refused on hot-reload (logged "restart coder to apply", previous
    /// state kept) — the `PathResolver` is the security boundary and is never
    /// rebuilt at runtime. Every OTHER field is a numeric tuning knob that
    /// hot-applies. Compared by value; the signature owns cloned copies.
    pub fn jail_signature(&self) -> JailSignature {
        JailSignature {
            base_path: self.base_path.clone(),
            base_paths: self.base_paths.clone(),
            non_accessible_globs: self.non_accessible_globs.clone(),
            default_exclude_globs: self.default_exclude_globs.clone(),
        }
    }
}

/// Expand `${NAME}` occurrences against the process environment.
/// Unknown variables expand to the empty string and emit a tracing warning.
/// Non-ASCII content outside `${...}` markers is preserved verbatim (the slice
/// boundary lands on the ASCII `$`, so this is UTF-8-safe), and an unterminated
/// `${` is treated as a literal.
fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        // Push the prefix verbatim (UTF-8-safe slice — start is a char boundary
        // because it points at an ASCII `$`).
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        tracing::warn!(var = %name, "config references undefined env var");
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                // Unterminated `${`; treat as literal.
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn load_config(path: &str) -> Result<CoderConfig> {
    let content = std::fs::read_to_string(path).with_context(|| format!("read {}", path))?;
    let cfg: CoderConfig =
        serde_yaml::from_str(&content).with_context(|| format!("parse {}", path))?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_yaml_parses_to_defaults() {
        let cfg: CoderConfig = serde_yaml::from_str("{}").expect("empty yaml parses");
        assert_eq!(cfg.base_path, None);
        assert!(cfg.base_paths.is_empty());
        assert!(cfg.non_accessible_globs.is_empty());
        assert_eq!(
            cfg.default_exclude_globs,
            vec![
                "**/.git/**",
                "**/node_modules/**",
                "**/target/**",
                "**/dist/**",
                "**/.venv/**",
                "**/__pycache__/**",
            ]
        );
        assert_eq!(cfg.max_read_bytes, 10 * 1024 * 1024);
        assert_eq!(cfg.max_write_bytes, 10 * 1024 * 1024);
        assert_eq!(cfg.tree_default_depth, 4);
        assert_eq!(cfg.tree_per_folder_limit, 50);
        assert_eq!(cfg.list_default_page_size, 100);
        assert_eq!(cfg.list_max_page_size, 1_000);
        assert_eq!(cfg.search_default_max_matches, 1_000);
        assert_eq!(cfg.search_default_max_line_bytes, 4_096);
        assert_eq!(cfg.batch_read_budget_bytes, 1_048_576);
        assert_eq!(cfg.max_output_bytes, 131_072);
        assert_eq!(cfg.search_response_budget_bytes, 262_144);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_yaml: CoderConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = CoderConfig::default();
        // Compare via JSON so we don't have to derive PartialEq.
        let a = serde_json::to_value(&from_yaml).unwrap();
        let b = serde_json::to_value(&from_default).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn legacy_base_path_parses_as_option() {
        let cfg: CoderConfig = serde_yaml::from_str("base_path: /tmp/legacy").unwrap();
        assert_eq!(cfg.base_path, Some(PathBuf::from("/tmp/legacy")));
        assert!(cfg.base_paths.is_empty());
    }

    #[test]
    fn custom_yaml_overrides_each_field() {
        let yaml = r#"
base_paths:
  - /tmp/c
  - /tmp/d
non_accessible_globs:
  - "**/.env"
default_exclude_globs:
  - "**/build/**"
max_read_bytes: 42
max_write_bytes: 43
tree_default_depth: 7
tree_per_folder_limit: 9
list_default_page_size: 11
list_max_page_size: 13
search_default_max_matches: 17
search_default_max_line_bytes: 19
batch_read_budget_bytes: 23
max_output_bytes: 31
search_response_budget_bytes: 29
"#;
        let cfg: CoderConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.base_path, None);
        assert_eq!(
            cfg.base_paths,
            vec![PathBuf::from("/tmp/c"), PathBuf::from("/tmp/d")]
        );
        assert_eq!(cfg.non_accessible_globs, vec!["**/.env".to_string()]);
        assert_eq!(cfg.default_exclude_globs, vec!["**/build/**".to_string()]);
        assert_eq!(cfg.max_read_bytes, 42);
        assert_eq!(cfg.max_write_bytes, 43);
        assert_eq!(cfg.tree_default_depth, 7);
        assert_eq!(cfg.tree_per_folder_limit, 9);
        assert_eq!(cfg.list_default_page_size, 11);
        assert_eq!(cfg.list_max_page_size, 13);
        assert_eq!(cfg.search_default_max_matches, 17);
        assert_eq!(cfg.search_default_max_line_bytes, 19);
        assert_eq!(cfg.batch_read_budget_bytes, 23);
        assert_eq!(cfg.max_output_bytes, 31);
        assert_eq!(cfg.search_response_budget_bytes, 29);
    }

    #[test]
    fn shipped_config_yaml_parses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml");
        let content = std::fs::read_to_string(path).expect("read config.yaml");
        let cfg: CoderConfig = serde_yaml::from_str(&content).expect("config.yaml parses");
        // T14: config.yaml now uses the multi-root form; legacy base_path is unset.
        assert_eq!(cfg.base_path, None);
        assert_eq!(
            cfg.base_paths,
            vec![PathBuf::from("./"), PathBuf::from("/tmp")]
        );
        assert!(cfg.non_accessible_globs.iter().any(|g| g.contains(".env")));
    }

    #[test]
    fn json_schema_has_expected_properties() {
        let schema = CoderConfig::json_schema();
        let props = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("schema has properties object");
        for field in [
            "base_path",
            "base_paths",
            "non_accessible_globs",
            "default_exclude_globs",
            "max_read_bytes",
            "max_write_bytes",
            "tree_default_depth",
            "tree_per_folder_limit",
            "list_default_page_size",
            "list_max_page_size",
            "search_default_max_matches",
            "search_default_max_line_bytes",
            "batch_read_budget_bytes",
            "max_output_bytes",
            "search_response_budget_bytes",
        ] {
            assert!(
                props.get(field).is_some(),
                "missing schema property {field}"
            );
        }
        // The field doc-comments survive as schema descriptions.
        assert!(props["non_accessible_globs"].get("description").is_some());
        // The `#[schemars(example = ...)]` attribute surfaces a top-level example.
        assert!(schema.get("example").is_some());
    }

    #[test]
    fn from_json_round_trips_from_default() {
        let cfg = CoderConfig::default();
        let json = cfg.to_json();
        let back = CoderConfig::from_json(&json).unwrap();
        let a = serde_json::to_value(&cfg).unwrap();
        let b = serde_json::to_value(&back).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn from_json_round_trips_custom_values() {
        let json = serde_json::json!({
            "base_paths": ["/tmp/x"],
            "non_accessible_globs": ["**/.env"],
            "max_read_bytes": 99,
            "tree_default_depth": 2,
        });
        let cfg = CoderConfig::from_json(&json).unwrap();
        assert_eq!(cfg.base_paths, vec![PathBuf::from("/tmp/x")]);
        assert_eq!(cfg.non_accessible_globs, vec!["**/.env".to_string()]);
        assert_eq!(cfg.max_read_bytes, 99);
        assert_eq!(cfg.tree_default_depth, 2);
        // Unspecified fields fall back to serde defaults.
        assert_eq!(cfg.max_write_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn from_json_tolerates_empty_object() {
        let back = CoderConfig::from_json(&serde_json::json!({})).unwrap();
        let a = serde_json::to_value(&back).unwrap();
        let b = serde_json::to_value(CoderConfig::default()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn from_json_rejects_garbage() {
        // Wrong type for a numeric field — serde rejects.
        let err = CoderConfig::from_json(&serde_json::json!({ "max_read_bytes": "not-a-number" }))
            .unwrap_err();
        assert!(err.contains("json parse"), "got: {err}");
        // A non-object value also fails.
        let err = CoderConfig::from_json(&serde_json::json!("garbage")).unwrap_err();
        assert!(err.contains("json parse"), "got: {err}");
    }

    #[test]
    fn to_json_round_trips_through_from_json() {
        let yaml = r#"
base_paths:
  - /tmp/a
max_output_bytes: 7
search_response_budget_bytes: 11
"#;
        let cfg = CoderConfig::from_yaml(yaml).unwrap();
        let back = CoderConfig::from_json(&cfg.to_json()).unwrap();
        assert_eq!(back.base_paths, vec![PathBuf::from("/tmp/a")]);
        assert_eq!(back.max_output_bytes, 7);
        assert_eq!(back.search_response_budget_bytes, 11);
    }

    #[test]
    fn from_yaml_expands_env_var() {
        std::env::set_var("CODER_TEST_ROOT", "/tmp/expanded-root");
        let yaml = "base_paths:\n  - \"${CODER_TEST_ROOT}\"\n";
        let cfg = CoderConfig::from_yaml(yaml).unwrap();
        assert_eq!(cfg.base_paths, vec![PathBuf::from("/tmp/expanded-root")]);
        std::env::remove_var("CODER_TEST_ROOT");
    }

    #[test]
    fn from_yaml_preserves_unicode_outside_markers() {
        // Guard against byte-iteration mojibake: a non-ASCII comment runs
        // through expand_env verbatim before serde_yaml strips it.
        let yaml = "# café 日本語\nmax_read_bytes: 5\n";
        let cfg = CoderConfig::from_yaml(yaml).unwrap();
        assert_eq!(cfg.max_read_bytes, 5);
    }

    #[test]
    fn jail_signature_equal_when_only_numeric_fields_differ() {
        // Two configs differing ONLY in numeric tuning knobs share a signature:
        // a hot-reload between them is allowed (no PathResolver rebuild needed).
        let base = CoderConfig {
            base_paths: vec![PathBuf::from("/tmp/a")],
            non_accessible_globs: vec!["**/.env".to_string()],
            default_exclude_globs: vec!["**/target/**".to_string()],
            ..CoderConfig::default()
        };
        let tuned = CoderConfig {
            max_read_bytes: base.max_read_bytes + 1,
            max_write_bytes: base.max_write_bytes + 1,
            tree_default_depth: base.tree_default_depth + 1,
            tree_per_folder_limit: base.tree_per_folder_limit + 1,
            list_default_page_size: base.list_default_page_size + 1,
            list_max_page_size: base.list_max_page_size + 1,
            search_default_max_matches: base.search_default_max_matches + 1,
            search_default_max_line_bytes: base.search_default_max_line_bytes + 1,
            batch_read_budget_bytes: base.batch_read_budget_bytes + 1,
            max_output_bytes: base.max_output_bytes + 1,
            search_response_budget_bytes: base.search_response_budget_bytes + 1,
            ..base.clone()
        };
        assert_eq!(base.jail_signature(), tuned.jail_signature());
    }

    #[test]
    fn jail_signature_differs_when_base_path_changes() {
        let a = CoderConfig::default();
        let b = CoderConfig {
            base_path: Some(PathBuf::from("/tmp/legacy")),
            ..CoderConfig::default()
        };
        assert_ne!(a.jail_signature(), b.jail_signature());
    }

    #[test]
    fn jail_signature_differs_when_base_paths_changes() {
        let a = CoderConfig {
            base_paths: vec![PathBuf::from("/tmp/a")],
            ..CoderConfig::default()
        };
        let b = CoderConfig {
            base_paths: vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")],
            ..CoderConfig::default()
        };
        assert_ne!(a.jail_signature(), b.jail_signature());
    }

    #[test]
    fn jail_signature_differs_when_non_accessible_globs_change() {
        let a = CoderConfig {
            non_accessible_globs: vec!["**/.env".to_string()],
            ..CoderConfig::default()
        };
        let b = CoderConfig {
            non_accessible_globs: vec!["**/.env".to_string(), "**/*.pem".to_string()],
            ..CoderConfig::default()
        };
        assert_ne!(a.jail_signature(), b.jail_signature());
    }

    #[test]
    fn jail_signature_differs_when_default_exclude_globs_change() {
        let a = CoderConfig {
            default_exclude_globs: vec!["**/target/**".to_string()],
            ..CoderConfig::default()
        };
        let b = CoderConfig {
            default_exclude_globs: vec!["**/build/**".to_string()],
            ..CoderConfig::default()
        };
        assert_ne!(a.jail_signature(), b.jail_signature());
    }
}
