//! Operator-facing runtime configuration.
//!
//! The authoritative value comes from the `configuration` worker at boot
//! (see [`crate::configuration`]); a `--config` YAML file, when passed,
//! only SEEDS the initial registration. Every field has a serde default
//! so an empty object yields a fully-populated config; per-call request
//! options override these defaults where the spec allows it (reserved
//! tokens, tail turns, prune thresholds).

use std::path::PathBuf;

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Root config shape. Unknown keys are rejected so a typo'd field
/// (e.g. `tail_truns: 3`) fails loudly instead of silently running the
/// default.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Cap on the default reserve: `min(cap, context_window * pct/100)`.
    #[serde(default = "default_reserved_tokens_cap")]
    pub reserved_tokens_cap: u64,

    /// Percentage of the context window reserved by default.
    #[serde(default = "default_reserved_pct")]
    pub reserved_pct: u64,

    /// user+assistant turn pairs kept verbatim by compaction when the
    /// request omits `options.tail_turns`.
    #[serde(default = "default_tail_turns")]
    pub tail_turns: usize,

    /// Newest function-output tokens never pruned (prune default).
    #[serde(default = "default_protect_recent_tokens")]
    pub protect_recent_tokens: u64,

    /// Skip pruning entirely when it would free fewer tokens than this.
    #[serde(default = "default_min_free_tokens")]
    pub min_free_tokens: u64,

    /// Per-output verbosity threshold (chars): outputs at or under this
    /// size are never considered verbose enough to prune.
    #[serde(default = "default_max_output_chars")]
    pub max_output_chars: usize,

    /// Compaction lease TTL in seconds.
    #[serde(default = "default_lease_ttl_secs")]
    pub lease_ttl_secs: u64,

    /// Fall back to conservative limits (8192/1024) when neither inline
    /// limits nor `llm-router` are available. When false the same
    /// situation errors with `could not resolve model limits`.
    #[serde(default = "default_allow_fallback_limits")]
    pub allow_fallback_limits: bool,

    /// Outer budget for one summariser call through `router::chat` (ms).
    #[serde(default = "default_summarizer_timeout_ms")]
    pub summarizer_timeout_ms: u64,

    /// Directory holding compaction-lease files (`<scope>/<key>.json`).
    /// A leading `~/` expands to the home directory. Mirrors
    /// session-manager's `data_dir`.
    #[serde(default = "default_lease_dir")]
    pub lease_dir: String,
}

impl WorkerConfig {
    /// The lease directory with a leading `~/` expanded to `$HOME`.
    pub fn resolved_lease_dir(&self) -> PathBuf {
        expand_tilde(&self.lease_dir)
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

    /// The restart-required fields: everything consumed ONCE at boot
    /// rather than per call. `summarizer_timeout_ms` is baked into the
    /// `RouterSummarizer` and `lease_dir` into the `FsLeaseStore`; a live
    /// config update that changes either is refused on hot-reload (logged
    /// "restart required", the previous snapshot kept). Every OTHER field
    /// is a per-call tuning knob that hot-applies.
    pub fn boot_signature(&self) -> BootSignature {
        BootSignature {
            lease_dir: self.lease_dir.clone(),
            summarizer_timeout_ms: self.summarizer_timeout_ms,
        }
    }
}

/// Signature of the config fields consumed at boot (see
/// [`WorkerConfig::boot_signature`]). Two configs with an equal signature
/// differ only in per-call tuning knobs that can be hot-applied; any
/// other difference requires a worker restart.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BootSignature {
    pub lease_dir: String,
    pub summarizer_timeout_ms: u64,
}

fn default_reserved_tokens_cap() -> u64 {
    20_000
}

fn default_reserved_pct() -> u64 {
    10
}

fn default_tail_turns() -> usize {
    2
}

fn default_protect_recent_tokens() -> u64 {
    40_000
}

fn default_min_free_tokens() -> u64 {
    20_000
}

fn default_max_output_chars() -> usize {
    2_000
}

fn default_lease_ttl_secs() -> u64 {
    300
}

fn default_allow_fallback_limits() -> bool {
    true
}

fn default_summarizer_timeout_ms() -> u64 {
    320_000
}

fn default_lease_dir() -> String {
    "~/.iii/data/context-manager".to_string()
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

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            reserved_tokens_cap: default_reserved_tokens_cap(),
            reserved_pct: default_reserved_pct(),
            tail_turns: default_tail_turns(),
            protect_recent_tokens: default_protect_recent_tokens(),
            min_free_tokens: default_min_free_tokens(),
            max_output_chars: default_max_output_chars(),
            lease_ttl_secs: default_lease_ttl_secs(),
            allow_fallback_limits: default_allow_fallback_limits(),
            summarizer_timeout_ms: default_summarizer_timeout_ms(),
            lease_dir: default_lease_dir(),
        }
    }
}

pub fn load_config(path: &str) -> Result<WorkerConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: WorkerConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.reserved_tokens_cap, 20_000);
        assert_eq!(cfg.reserved_pct, 10);
        assert_eq!(cfg.tail_turns, 2);
        assert_eq!(cfg.protect_recent_tokens, 40_000);
        assert_eq!(cfg.min_free_tokens, 20_000);
        assert_eq!(cfg.max_output_chars, 2_000);
        assert_eq!(cfg.lease_ttl_secs, 300);
        assert!(cfg.allow_fallback_limits);
        assert_eq!(cfg.summarizer_timeout_ms, 320_000);
        assert_eq!(cfg.lease_dir, "~/.iii/data/context-manager");
    }

    #[test]
    fn custom_yaml_overrides_every_field() {
        let yaml = "reserved_tokens_cap: 1\nreserved_pct: 2\ntail_turns: 3\n\
                    protect_recent_tokens: 4\nmin_free_tokens: 5\nmax_output_chars: 6\n\
                    lease_ttl_secs: 7\nallow_fallback_limits: false\nsummarizer_timeout_ms: 8\n\
                    lease_dir: /tmp/leases";
        let cfg: WorkerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.reserved_tokens_cap, 1);
        assert_eq!(cfg.reserved_pct, 2);
        assert_eq!(cfg.tail_turns, 3);
        assert_eq!(cfg.protect_recent_tokens, 4);
        assert_eq!(cfg.min_free_tokens, 5);
        assert_eq!(cfg.max_output_chars, 6);
        assert_eq!(cfg.lease_ttl_secs, 7);
        assert!(!cfg.allow_fallback_limits);
        assert_eq!(cfg.summarizer_timeout_ms, 8);
        assert_eq!(cfg.lease_dir, "/tmp/leases");
        assert_eq!(cfg.resolved_lease_dir(), PathBuf::from("/tmp/leases"));
    }

    #[test]
    fn lease_dir_tilde_expands_to_home() {
        let cfg = WorkerConfig::default();
        let resolved = cfg.resolved_lease_dir();
        if let Some(home) = dirs::home_dir() {
            assert!(resolved.starts_with(home));
            assert!(resolved.ends_with(".iii/data/context-manager"));
        }
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_yaml: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(from_yaml, WorkerConfig::default());
    }

    #[test]
    fn unknown_root_key_is_rejected_at_parse() {
        let err = serde_yaml::from_str::<WorkerConfig>("tail_truns: 3").unwrap_err();
        assert!(err.to_string().contains("unknown field"), "got: {err}");
    }

    #[test]
    fn committed_config_yaml_parses_to_defaults() {
        let cfg = load_config(concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml")).unwrap();
        assert_eq!(cfg, WorkerConfig::default());
    }

    #[test]
    fn json_schema_has_every_property_with_descriptions_and_example() {
        let schema = WorkerConfig::json_schema();
        let props = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("schema has a properties object");
        for field in [
            "reserved_tokens_cap",
            "reserved_pct",
            "tail_turns",
            "protect_recent_tokens",
            "min_free_tokens",
            "max_output_chars",
            "lease_ttl_secs",
            "allow_fallback_limits",
            "summarizer_timeout_ms",
            "lease_dir",
        ] {
            assert!(
                props.get(field).is_some(),
                "missing schema property {field}"
            );
        }
        // Field doc-comments survive as schema descriptions.
        assert!(props["lease_dir"].get("description").is_some());
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
        let json = serde_json::json!({ "tail_turns": 5, "lease_dir": "/tmp/x" });
        let cfg = WorkerConfig::from_json(&json).unwrap();
        assert_eq!(cfg.tail_turns, 5);
        assert_eq!(cfg.lease_dir, "/tmp/x");
        // Unspecified fields fall back to serde defaults.
        assert_eq!(cfg.summarizer_timeout_ms, 320_000);
    }

    #[test]
    fn from_json_rejects_garbage() {
        let err = WorkerConfig::from_json(&serde_json::json!({ "tail_turns": "nan" })).unwrap_err();
        assert!(err.contains("json parse"), "got: {err}");
        let err = WorkerConfig::from_json(&serde_json::json!("garbage")).unwrap_err();
        assert!(err.contains("json parse"), "got: {err}");
    }

    #[test]
    fn from_yaml_expands_env_var() {
        std::env::set_var("CTX_MGR_TEST_LEASE_DIR", "/tmp/expanded-leases");
        let cfg = WorkerConfig::from_yaml("lease_dir: \"${CTX_MGR_TEST_LEASE_DIR}\"\n").unwrap();
        assert_eq!(cfg.lease_dir, "/tmp/expanded-leases");
        std::env::remove_var("CTX_MGR_TEST_LEASE_DIR");
    }

    #[test]
    fn boot_signature_equal_when_only_tuning_knobs_differ() {
        let base = WorkerConfig::default();
        let tuned = WorkerConfig {
            tail_turns: base.tail_turns + 1,
            reserved_pct: base.reserved_pct + 1,
            protect_recent_tokens: base.protect_recent_tokens + 1,
            ..base.clone()
        };
        assert_eq!(base.boot_signature(), tuned.boot_signature());
    }

    #[test]
    fn boot_signature_differs_on_restart_required_fields() {
        let base = WorkerConfig::default();
        let moved_dir = WorkerConfig {
            lease_dir: "/tmp/other".to_string(),
            ..base.clone()
        };
        let new_timeout = WorkerConfig {
            summarizer_timeout_ms: base.summarizer_timeout_ms + 1,
            ..base.clone()
        };
        assert_ne!(base.boot_signature(), moved_dir.boot_signature());
        assert_ne!(base.boot_signature(), new_timeout.boot_signature());
    }
}
