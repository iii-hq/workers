//! Operator config, served by the `configuration` worker (docs/sops/configuration.md).
//!
//! One flat struct rather than nested `node:`/`python:` sections: the three
//! shared knobs would otherwise need duplicating or hoisting, and flat is what
//! both engines already ship.
//!
//! `deny_unknown_fields` is deliberate, following python-engine rather than
//! node-engine: this is operator config, not a wire request, so a typo like
//! `max_runtimez: 3` should fail loudly rather than silently no-op.
//!
//! Reload semantics (Tier 1, hot snapshot via [`SharedConfig`]): the output
//! caps (`max_result_bytes`/`max_stream_bytes`) and the timeout knobs are read
//! per call and hot-reload on `configuration:updated`. The engine-structural
//! fields — `max_runtimes`, `heap_mb`, `external_mb`, `idle_ttl_secs`, and the
//! scratch knobs — are captured when the V8/CPython engines are built at boot
//! and CANNOT be rebound under live guest runtimes; a change is persisted
//! immediately but applies at the next worker restart (the reload handler says
//! so in the log). This is the documented exception the SOP's hot-reload rule
//! allows.

use std::sync::Arc;

use anyhow::Result;
use arc_swap::ArcSwap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Hot-swappable config snapshot: handlers `load()` per call (lock-free),
/// the configuration trigger `store()`s a whole new value.
pub type SharedConfig = Arc<ArcSwap<CodeRunnerConfig>>;

fn default_max_result_bytes() -> usize {
    32_768
}

fn default_max_stream_bytes() -> usize {
    16_384
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CodeRunnerConfig {
    /// Live runtimes across both engines. Applied at worker start.
    pub max_runtimes: usize,
    /// Timeout used when a run omits `timeout_ms`.
    pub default_timeout_ms: u64,
    /// Hard ceiling a run's `timeout_ms` is clamped to.
    pub max_timeout_ms: u64,
    /// Reap runtimes idle longer than this. Applied at worker start.
    pub idle_ttl_secs: u64,
    /// V8 object-heap cap per node runtime (MiB). Applied at worker start.
    pub heap_mb: usize,
    /// Off-heap cap per node runtime (ArrayBuffer/TypedArray, MiB); `heap_mb`
    /// does not cover it. Applied at worker start.
    pub external_mb: usize,
    /// Per-runtime scratch directory (`iii.files`), MiB. 0 disables it
    /// and removes the guest surface. Applied at worker start.
    ///
    /// WORST-CASE HOST FOOTPRINT IS `max_runtimes * scratch_mb`, and the
    /// system temp directory is tmpfs — host RAM — on most Linux hosts.
    pub scratch_mb: usize,
    /// Maximum files per scratch directory. Applied at worker start.
    pub scratch_files: usize,
    /// Where scratch directories live; unset uses the system temp directory.
    /// Applied at worker start.
    pub scratch_root: Option<String>,
    /// Byte ceiling for the serialized `result` echoed in a RunResponse;
    /// larger values are replaced by an omission marker string. 0 disables.
    #[serde(default = "default_max_result_bytes")]
    pub max_result_bytes: usize,
    /// Byte ceiling for each of stdout/stderr; larger streams keep
    /// head+tail around a truncation marker. 0 disables.
    #[serde(default = "default_max_stream_bytes")]
    pub max_stream_bytes: usize,
}

impl Default for CodeRunnerConfig {
    fn default() -> Self {
        Self {
            max_runtimes: 32,
            default_timeout_ms: 5_000,
            max_timeout_ms: 30_000,
            idle_ttl_secs: 900,
            heap_mb: 128,
            external_mb: 64,
            scratch_mb: 8,
            scratch_files: 64,
            scratch_root: None,
            max_result_bytes: default_max_result_bytes(),
            max_stream_bytes: default_max_stream_bytes(),
        }
    }
}

impl CodeRunnerConfig {
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(CodeRunnerConfig))
            .expect("CodeRunnerConfig schema serializes")
    }

    /// Parse a value already env-expanded by the configuration worker.
    pub fn from_json(v: &serde_json::Value) -> Result<CodeRunnerConfig, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("invalid code-runner config: {e}"))
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("CodeRunnerConfig serializes")
    }

    pub fn into_shared(self) -> SharedConfig {
        Arc::new(ArcSwap::from_pointee(self))
    }

    /// True when `self` → `next` changes a field the engines captured at
    /// boot — everything except the per-call knobs (output caps + timeouts).
    /// The reload handler uses this to say "applies at next restart".
    pub fn restart_required(&self, next: &CodeRunnerConfig) -> bool {
        let per_call = |c: &CodeRunnerConfig| CodeRunnerConfig {
            max_result_bytes: 0,
            max_stream_bytes: 0,
            default_timeout_ms: 0,
            max_timeout_ms: 0,
            ..c.clone()
        };
        per_call(self) != per_call(next)
    }

    /// The node engine's own config, derived from this one.
    pub fn node(&self) -> iii_node_core::config::NodeEngineConfig {
        iii_node_core::config::NodeEngineConfig {
            max_runtimes: self.max_runtimes,
            default_timeout_ms: self.default_timeout_ms,
            max_timeout_ms: self.max_timeout_ms,
            heap_mb: self.heap_mb,
            external_mb: self.external_mb,
            idle_ttl_secs: self.idle_ttl_secs,
            scratch_mb: self.scratch_mb,
            scratch_files: self.scratch_files,
            scratch_root: self.scratch_root.clone(),
        }
    }

    /// The python half of the same operator config.
    ///
    /// Every key this worker documents and both engines understand is passed
    /// through. Without this the python engine silently ran on its own
    /// defaults — an operator lowering `idle_ttl_secs` or `max_timeout_ms`
    /// changed node's behaviour and nothing else, with no error to say so.
    ///
    /// Two keys are NOT taken from the shared config, on purpose:
    /// `max_concurrent_runs` bounds how many CPython instances execute at
    /// once and is a memory ceiling of a different shape to node's isolate
    /// count, and the memory knobs are python's own — `heap_mb`/`external_mb`
    /// are V8 concepts with no wasm equivalent.
    pub fn python(&self) -> iii_python_core::config::PythonEngineConfig {
        let d = iii_python_core::config::PythonEngineConfig::default();
        iii_python_core::config::PythonEngineConfig {
            max_runtimes: self.max_runtimes,
            idle_ttl_secs: self.idle_ttl_secs,
            default_timeout_ms: self.default_timeout_ms,
            max_timeout_ms: self.max_timeout_ms,
            ..d
        }
    }
}

/// Parse an optional `--config` seed file. Used ONLY as a one-time
/// `initial_value` on first registration with the configuration worker —
/// after that, the configuration worker's stored value is authoritative and
/// nothing on disk is read at runtime.
pub fn load_config(path: &str) -> Result<CodeRunnerConfig> {
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_yaml::from_str(&raw)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_config_keys_are_rejected() {
        let err = serde_yaml::from_str::<CodeRunnerConfig>("max_runtimez: 3").unwrap_err();
        assert!(err.to_string().contains("max_runtimez"));
    }

    /// Every shared knob must reach the engine that enforces it. Mutation:
    /// drop any line from `node()` and the corresponding assertion fails.
    #[test]
    fn the_node_config_carries_every_shared_knob() {
        let cfg = CodeRunnerConfig {
            max_runtimes: 3,
            default_timeout_ms: 11,
            max_timeout_ms: 22,
            idle_ttl_secs: 33,
            heap_mb: 44,
            external_mb: 55,
            scratch_mb: 66,
            scratch_files: 77,
            scratch_root: Some("/tmp/x".into()),
            ..Default::default()
        };
        let n = cfg.node();
        assert_eq!(n.max_runtimes, 3);
        assert_eq!(n.default_timeout_ms, 11);
        assert_eq!(n.max_timeout_ms, 22);
        assert_eq!(n.idle_ttl_secs, 33);
        assert_eq!(n.heap_mb, 44);
        assert_eq!(n.external_mb, 55);
        assert_eq!(n.scratch_mb, 66);
        assert_eq!(n.scratch_files, 77);
        assert_eq!(n.scratch_root.as_deref(), Some("/tmp/x"));
    }

    #[test]
    fn output_caps_default_on() {
        let c = CodeRunnerConfig::default();
        assert_eq!(c.max_result_bytes, 32_768);
        assert_eq!(c.max_stream_bytes, 16_384);
    }

    /// What `configuration::register` publishes and `configuration::set`
    /// validates against must round-trip the defaults exactly.
    #[test]
    fn json_round_trips_the_defaults() {
        let d = CodeRunnerConfig::default();
        assert_eq!(CodeRunnerConfig::from_json(&d.to_json()).unwrap(), d);
    }

    #[test]
    fn schema_names_every_field() {
        let schema = CodeRunnerConfig::json_schema();
        let props = schema["properties"].as_object().unwrap();
        for field in [
            "max_runtimes",
            "default_timeout_ms",
            "max_timeout_ms",
            "idle_ttl_secs",
            "heap_mb",
            "external_mb",
            "scratch_mb",
            "scratch_files",
            "scratch_root",
            "max_result_bytes",
            "max_stream_bytes",
        ] {
            assert!(props.contains_key(field), "schema lost `{field}`");
        }
    }

    /// The reload handler's restart warning must fire for boot-captured
    /// fields ONLY — a caps or timeout change is fully hot.
    #[test]
    fn restart_required_splits_hot_from_boot_fields() {
        let base = CodeRunnerConfig::default();
        let hot = CodeRunnerConfig {
            max_result_bytes: 1,
            max_stream_bytes: 2,
            default_timeout_ms: 3,
            max_timeout_ms: 4,
            ..base.clone()
        };
        assert!(!base.restart_required(&hot));
        let boot = CodeRunnerConfig {
            max_runtimes: base.max_runtimes + 1,
            ..base.clone()
        };
        assert!(base.restart_required(&boot));
    }
}
