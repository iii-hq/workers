//! Runtime caps and operator-facing limits, plus the compile-time caps the
//! wrapper, runner and manager enforce against. Unlike `NodeEngineConfig`,
//! this rejects unknown keys: it's operator config, not a wire request, so a
//! typo like `max_runtimez: 3` should fail loudly rather than silently no-op.
//!
//! Deserialization lives here; READING a file does not. `load_config` stays
//! with the worker that owns a `config.yaml`, which is what keeps `serde_yaml`
//! out of this crate.
use serde::Deserialize;

pub const MAX_CODE_BYTES: usize = 1_048_576;
pub const MAX_PAYLOAD_BYTES: usize = 1_048_576;
pub const MAX_RESULT_BYTES: usize = 1_048_576;
pub const MAX_LOG_LINES: usize = 1_000;
pub const MAX_LOG_BYTES: usize = 262_144;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PythonEngineConfig {
    pub max_concurrent_runs: usize,
    /// Live runtimes (each owning a persistent `/work`) allowed at once.
    ///
    /// Deliberately far below node's 32: a python runtime's directory budget
    /// is `MAX_WORK_DIR_BYTES`, so the host ceiling here is
    /// `max_runtimes * 256 MiB` — 2 GiB at this default, and on a tmpfs
    /// `$TMPDIR` that is host RAM.
    pub max_runtimes: usize,
    /// Reap a runtime with no activity for this long. The only backstop for
    /// a caller that never tears one down.
    pub idle_ttl_secs: u64,
    pub default_timeout_ms: u64,
    pub max_timeout_ms: u64,
    pub default_memory_mb: u64,
    pub max_memory_mb: u64,
}

impl Default for PythonEngineConfig {
    fn default() -> Self {
        Self {
            max_concurrent_runs: 8,
            max_runtimes: 8,
            idle_ttl_secs: 900,
            default_timeout_ms: 5_000,
            max_timeout_ms: 30_000,
            default_memory_mb: 128,
            max_memory_mb: 512,
        }
    }
}

impl PythonEngineConfig {
    // Ceilings are floored to 1: `u64::clamp` PANICS when min > max, and a
    // configured `max_timeout_ms: 0` / `max_memory_mb: 0` reaches here
    // unvalidated — that must degrade to the tightest real ceiling, not
    // abort every run.
    pub fn clamp_timeout(&self, requested: Option<u64>) -> u64 {
        requested
            .unwrap_or(self.default_timeout_ms)
            .clamp(1, self.max_timeout_ms.max(1))
    }
    pub fn clamp_memory(&self, requested: Option<u64>) -> u64 {
        requested
            .unwrap_or(self.default_memory_mb)
            .clamp(1, self.max_memory_mb.max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_apply_defaults_and_ceilings() {
        let cfg = PythonEngineConfig::default();
        assert_eq!(cfg.clamp_timeout(None), 5_000);
        assert_eq!(cfg.clamp_timeout(Some(120_000)), 30_000);
        assert_eq!(cfg.clamp_timeout(Some(0)), 1);
        assert_eq!(cfg.clamp_memory(None), 128);
        assert_eq!(cfg.clamp_memory(Some(4_096)), 512);
    }

    /// A zero ceiling is an operator mistake, not a licence to panic:
    /// `clamp(1, 0)` aborts every run. Mutation: drop either `.max(1)`.
    #[test]
    fn a_zero_ceiling_degrades_instead_of_panicking() {
        let cfg = PythonEngineConfig {
            max_timeout_ms: 0,
            max_memory_mb: 0,
            ..PythonEngineConfig::default()
        };
        assert_eq!(cfg.clamp_timeout(Some(5_000)), 1);
        assert_eq!(cfg.clamp_memory(Some(256)), 1);
    }
}
