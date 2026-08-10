//! Operator-authored config.
//!
//! One flat struct rather than nested `node:`/`python:` sections: the three
//! shared knobs would otherwise need duplicating or hoisting, and flat is what
//! both engines already ship.
//!
//! `deny_unknown_fields` is deliberate, following python-engine rather than
//! node-engine: this is operator config, not a wire request, so a typo like
//! `max_runtimez: 3` should fail loudly rather than silently no-op.

use anyhow::Result;
use serde::Deserialize;

fn default_max_result_bytes() -> usize {
    32_768
}

fn default_max_stream_bytes() -> usize {
    16_384
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CodeRunnerConfig {
    /// Live runtimes across both engines.
    pub max_runtimes: usize,
    pub default_timeout_ms: u64,
    pub max_timeout_ms: u64,
    pub idle_ttl_secs: u64,
    /// V8 object-heap cap per node runtime.
    pub heap_mb: usize,
    /// Off-heap cap per node runtime (ArrayBuffer/TypedArray); `heap_mb` does
    /// not cover it.
    pub external_mb: usize,
    /// Per-runtime scratch directory (`iii.files`), total bytes. 0 disables it
    /// and removes the guest surface.
    ///
    /// WORST-CASE HOST FOOTPRINT IS `max_runtimes * scratch_mb`, and the
    /// system temp directory is tmpfs — host RAM — on most Linux hosts.
    pub scratch_mb: usize,
    pub scratch_files: usize,
    /// Where scratch directories live; `None` uses the system temp directory.
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
    fn committed_config_yaml_matches_struct_defaults() {
        let cfg = load_config("config.yaml").expect("committed config.yaml parses");
        assert_eq!(cfg, CodeRunnerConfig::default());
    }

    #[test]
    fn output_caps_default_on() {
        let c = CodeRunnerConfig::default();
        assert_eq!(c.max_result_bytes, 32_768);
        assert_eq!(c.max_stream_bytes, 16_384);
    }
}
