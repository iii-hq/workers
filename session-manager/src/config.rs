//! Operator-facing runtime configuration registered with the `configuration`
//! worker (id `session-manager`). The authoritative value is fetched at boot
//! via `configuration::get`; built-in defaults in [`WorkerConfig::default`]
//! seed the first registration when nothing is stored yet. An optional
//! `--config` YAML path may override that seed on first boot only.
//!
//! The storage adapter is selected by an `adapter` block: a `name`
//! discriminator plus a nested, adapter-specific `config` object (the same
//! shape other iii workers like `iii-state` use):
//!
//! ```yaml
//! adapter:
//!   name: fs
//!   config:
//!     data_dir: ~/.iii/data/session-manager
//! ```
//!
//! or
//!
//! ```yaml
//! adapter:
//!   name: bridge
//!   config:
//!     url: ws://127.0.0.1:49134
//!     timeout_ms: 5000
//! ```
//!
//! `adapter` is an adjacently tagged enum (`name` + `config`), so `schemars`
//! emits a JSON Schema `oneOf` the console renders as a variant picker plus
//! the fields for the selected adapter. Each variant's `config` is a typed
//! struct (`deny_unknown_fields`), so a bridge without a `url` — or a typo'd
//! key — fails at parse time, before the worker ever tries to boot (a
//! misconfigured bridge must never silently fall back to a local fs store).

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Root config shape. Unknown keys are rejected so a typo'd field
/// (e.g. `adaptr:`) fails loudly instead of silently running the default
/// adapter.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Storage adapter selection (`fs` or `bridge`) plus its settings.
    /// Default: `fs` with the standard data directory.
    #[serde(default)]
    pub adapter: StorageAdapter,

    /// Default page size for `session::list` / `session::messages` when
    /// the request omits `limit`.
    #[serde(default = "default_default_list_limit")]
    pub default_list_limit: usize,

    /// Hard cap applied to any requested `limit`.
    #[serde(default = "default_max_list_limit")]
    pub max_list_limit: usize,
}

/// Storage adapter selection. Adjacently tagged (`name` + `config`) so the
/// wire shape mirrors other iii workers and `schemars` emits a `oneOf` the
/// console renders as a variant picker exposing only the selected adapter's
/// fields. Each variant's `config` is validated at parse time.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(tag = "name", content = "config", rename_all = "snake_case")]
pub enum StorageAdapter {
    /// One append-only JSONL file per session under `data_dir`.
    Fs(FsBackendConfig),
    /// Defer raw storage (and event fan-out) to a main session-manager on
    /// another iii instance.
    Bridge(BridgeBackendConfig),
}

impl Default for StorageAdapter {
    fn default() -> Self {
        StorageAdapter::Fs(FsBackendConfig::default())
    }
}

/// Settings for the `fs` adapter.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FsBackendConfig {
    /// Directory holding one `<session_id>.jsonl` per session. A
    /// leading `~/` expands to the home directory.
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
}

impl FsBackendConfig {
    pub fn resolved_data_dir(&self) -> PathBuf {
        expand_tilde(&self.data_dir)
    }
}

impl Default for FsBackendConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
        }
    }
}

/// Settings for the `bridge` adapter.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BridgeBackendConfig {
    /// WebSocket URL of the main instance's engine.
    pub url: String,

    /// Per store/publish call timeout (milliseconds).
    #[serde(default = "default_bridge_timeout_ms")]
    pub timeout_ms: u64,
}

impl WorkerConfig {
    /// The validated storage adapter. Parsing already enforces each adapter's
    /// shape (`fs` needs nothing; `bridge` requires `url`), so this is simply
    /// the typed selection cloned for the boot wiring in `main.rs`.
    pub fn resolve_adapter(&self) -> StorageAdapter {
        self.adapter.clone()
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
    /// attached as a top-level `example`. `adapter` is an adjacently tagged
    /// enum, so it renders as a `oneOf` (a variant picker plus the selected
    /// adapter's fields) in the console; the per-adapter `config` shape is
    /// validated at parse time by serde.
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

    /// The adapter-defining fields: everything the storage `SessionStore` and
    /// event sink are built from. The reload path compares this to decide
    /// whether a config change needs a full runtime rebuild (the `adapter`
    /// differs) or only a per-call list-limit swap (it matches). Both paths
    /// apply live — nothing requires a restart.
    pub fn boot_signature(&self) -> BootSignature {
        BootSignature {
            adapter: self.adapter.clone(),
        }
    }
}

/// Signature of the adapter-defining config fields (see
/// [`WorkerConfig::boot_signature`]). Two configs with an equal signature
/// differ only in the per-call list limits (a cheap snapshot swap); any other
/// difference (an adapter swap, a new `data_dir`, a bridge URL/timeout change)
/// rebuilds and hot-swaps the runtime.
#[derive(Clone, Debug, PartialEq)]
pub struct BootSignature {
    pub adapter: StorageAdapter,
}

pub fn default_data_dir() -> String {
    "~/.iii/data/session-manager".to_string()
}

fn default_bridge_timeout_ms() -> u64 {
    5000
}

fn default_default_list_limit() -> usize {
    50
}

fn default_max_list_limit() -> usize {
    500
}

fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Expand `${NAME}` occurrences against the process environment. Unknown
/// variables expand to the empty string and emit a tracing warning. Only the
/// `--config` seed path uses this — values from `configuration::get` are
/// already expanded by the worker. A `${VAR:default}` placeholder (the
/// configuration worker's own syntax) is left untouched for the worker to
/// expand on read; an unterminated `${` is kept as a literal.
fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        if name.contains(':') {
            // `${VAR:default}` belongs to the configuration worker; preserve it.
            out.push_str(&rest[start..start + 2 + end + 1]);
        } else {
            match std::env::var(name) {
                Ok(val) => out.push_str(&val),
                Err(_) => tracing::warn!(
                    var = %name,
                    "seed config references unset env var; expanding to empty string"
                ),
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            adapter: StorageAdapter::default(),
            default_list_limit: default_default_list_limit(),
            max_list_limit: default_max_list_limit(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml_resolve_to_fs() {
        let cfg: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.default_list_limit, 50);
        assert_eq!(cfg.max_list_limit, 500);
        let StorageAdapter::Fs(fs) = cfg.resolve_adapter() else {
            panic!("expected fs adapter");
        };
        assert_eq!(fs.data_dir, "~/.iii/data/session-manager");
    }

    #[test]
    fn fs_yaml_overrides_data_dir() {
        let yaml = "adapter:\n  name: fs\n  config:\n    data_dir: /tmp/sessions";
        let cfg: WorkerConfig = serde_yaml::from_str(yaml).unwrap();
        let StorageAdapter::Fs(fs) = cfg.resolve_adapter() else {
            panic!("expected fs adapter");
        };
        assert_eq!(fs.data_dir, "/tmp/sessions");
        assert_eq!(fs.resolved_data_dir(), PathBuf::from("/tmp/sessions"));
    }

    #[test]
    fn tilde_data_dir_expands_to_home() {
        let fs = FsBackendConfig::default();
        let resolved = fs.resolved_data_dir();
        if let Some(home) = dirs::home_dir() {
            assert!(resolved.starts_with(home));
            assert!(resolved.ends_with(".iii/data/session-manager"));
        }
    }

    #[test]
    fn bridge_yaml_parses_with_defaulted_timeout() {
        let yaml = "adapter:\n  name: bridge\n  config:\n    url: ws://main:49134";
        let cfg: WorkerConfig = serde_yaml::from_str(yaml).unwrap();
        let StorageAdapter::Bridge(bridge) = cfg.resolve_adapter() else {
            panic!("expected bridge adapter");
        };
        assert_eq!(bridge.url, "ws://main:49134");
        assert_eq!(bridge.timeout_ms, 5000);
    }

    #[test]
    fn bridge_without_url_is_rejected_at_parse() {
        // `url` is required by `BridgeBackendConfig`, so a bridge adapter
        // without it fails at parse time rather than silently at boot.
        let yaml = "adapter:\n  name: bridge\n  config:\n    timeout_ms: 100";
        let err = serde_yaml::from_str::<WorkerConfig>(yaml).unwrap_err();
        assert!(err.to_string().contains("url"), "got: {err}");
    }

    #[test]
    fn unknown_adapter_config_key_is_rejected_at_parse() {
        // Each adapter `config` is `deny_unknown_fields`, so a typo'd key
        // fails at parse time.
        let yaml = "adapter:\n  name: fs\n  config:\n    datadir: /tmp/x";
        let err = serde_yaml::from_str::<WorkerConfig>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"), "got: {err}");
    }

    #[test]
    fn unknown_adapter_name_is_rejected_at_parse() {
        let yaml = "adapter:\n  name: cloud\n  config: {}";
        assert!(serde_yaml::from_str::<WorkerConfig>(yaml).is_err());
    }

    #[test]
    fn unknown_root_key_is_rejected_at_parse() {
        let err = serde_yaml::from_str::<WorkerConfig>("backnd: bridge").unwrap_err();
        assert!(err.to_string().contains("unknown field"), "got: {err}");
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_yaml: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = WorkerConfig::default();
        assert_eq!(from_yaml, from_default);
    }

    #[test]
    fn json_roundtrips_through_to_from() {
        let cfg = WorkerConfig {
            default_list_limit: 7,
            max_list_limit: 99,
            ..WorkerConfig::default()
        };
        let back = WorkerConfig::from_json(&cfg.to_json()).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn from_json_empty_object_uses_defaults() {
        let cfg = WorkerConfig::from_json(&serde_json::json!({})).unwrap();
        assert_eq!(cfg, WorkerConfig::default());
    }

    #[test]
    fn json_schema_carries_a_default_example() {
        let schema = WorkerConfig::json_schema();
        assert_eq!(schema["example"], WorkerConfig::default().to_json());
    }

    #[test]
    fn json_schema_exposes_adapter_oneof_variants() {
        // The console renders a variant picker plus the selected adapter's
        // fields only when `adapter` is a `oneOf` of typed objects keyed by
        // `name`. Assert the shape so it can never regress to the read-only
        // "unsupported schema" fallback.
        let schema = WorkerConfig::json_schema();
        let adapter = &schema["definitions"]["StorageAdapter"];
        let variants = adapter["oneOf"]
            .as_array()
            .unwrap_or_else(|| panic!("StorageAdapter schema is a oneOf; got: {adapter}"));
        assert_eq!(variants.len(), 2, "fs + bridge variants: {adapter}");
        let names: Vec<&str> = variants
            .iter()
            .filter_map(|v| v["properties"]["name"]["enum"][0].as_str())
            .collect();
        assert!(names.contains(&"fs"), "missing fs variant: {adapter}");
        assert!(
            names.contains(&"bridge"),
            "missing bridge variant: {adapter}"
        );
    }

    #[test]
    fn boot_signature_matches_for_tuning_only_change() {
        let boot = WorkerConfig::default();
        let tuned = WorkerConfig {
            default_list_limit: boot.default_list_limit + 1,
            max_list_limit: boot.max_list_limit + 1,
            ..boot.clone()
        };
        assert_eq!(tuned.boot_signature(), boot.boot_signature());
    }

    #[test]
    fn boot_signature_differs_when_adapter_changes() {
        let boot = WorkerConfig::default();
        let bridged = WorkerConfig {
            adapter: StorageAdapter::Bridge(BridgeBackendConfig {
                url: "ws://main:49134".to_string(),
                timeout_ms: default_bridge_timeout_ms(),
            }),
            ..boot.clone()
        };
        assert_ne!(bridged.boot_signature(), boot.boot_signature());
    }

    #[test]
    fn boot_signature_differs_when_adapter_config_changes() {
        let boot = WorkerConfig::default();
        let moved = WorkerConfig {
            adapter: StorageAdapter::Fs(FsBackendConfig {
                data_dir: "/tmp/elsewhere".to_string(),
            }),
            ..boot.clone()
        };
        assert_ne!(moved.boot_signature(), boot.boot_signature());
    }

    #[test]
    fn from_yaml_expands_env_and_preserves_worker_placeholders() {
        std::env::set_var("SM_TEST_LIMIT", "123");
        let yaml = "default_list_limit: ${SM_TEST_LIMIT}\nadapter:\n  name: fs\n  config:\n    data_dir: \"${SM_UNSET_DIR:/var/data}\"";
        let cfg = WorkerConfig::from_yaml(yaml).unwrap();
        assert_eq!(cfg.default_list_limit, 123);
        // `${VAR:default}` is left for the configuration worker to expand.
        let StorageAdapter::Fs(fs) = cfg.resolve_adapter() else {
            panic!("expected fs adapter");
        };
        assert_eq!(fs.data_dir, "${SM_UNSET_DIR:/var/data}");
        std::env::remove_var("SM_TEST_LIMIT");
    }
}
