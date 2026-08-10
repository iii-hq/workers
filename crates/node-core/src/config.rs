use std::time::Duration;

use serde::Deserialize;

/// Operator-facing runtime limits. Every field has a `serde(default)` so an
/// empty or partial config still yields a fully-populated struct.
///
/// Deserialization lives here; READING a file does not. `load_config` stays
/// with the worker that owns a `config.yaml`, which is what keeps `serde_yaml`
/// and `anyhow` out of this crate.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NodeEngineConfig {
    #[serde(default = "default_max_runtimes")]
    pub max_runtimes: usize,
    #[serde(default = "default_default_timeout_ms")]
    pub default_timeout_ms: u64,
    #[serde(default = "default_max_timeout_ms")]
    pub max_timeout_ms: u64,
    #[serde(default = "default_heap_mb")]
    pub heap_mb: usize,
    /// Ceiling on memory allocated OUTSIDE the V8 object heap — every
    /// `ArrayBuffer`, and so every `TypedArray`. `heap_mb` does not cover it:
    /// the two are accounted separately, so without this a runtime could hold
    /// hundreds of megabytes resident and still OOM correctly on its object
    /// heap. Exceeding it kills that runtime with `node-engine::oom`, the same
    /// way exceeding `heap_mb` does; the worker and other runtimes carry on.
    #[serde(default = "default_external_mb")]
    pub external_mb: usize,
    #[serde(default = "default_idle_ttl_secs")]
    pub idle_ttl_secs: u64,
    /// Per-runtime private scratch directory (`iii.files`), total bytes.
    ///
    /// 0 disables the feature entirely: no directory is created and
    /// `iii.files` is absent from the guest surface. That kill switch exists
    /// because this is the FIRST filesystem this sandbox has ever had — an
    /// operator upgrading would otherwise get one they never asked for, on
    /// every existing tenant's isolate, from a version bump.
    ///
    /// Worst-case host footprint is `max_runtimes * scratch_mb` — 256 MiB at
    /// the defaults. That matters because the system temp directory is tmpfs
    /// (host RAM) on most Linux hosts; see `scratch_root` to move it.
    ///
    /// Deliberately NOT cross-validated against `external_mb`: a read of a
    /// max-size file allocates that many bytes in V8, so a `scratch_mb` above
    /// `external_mb` makes such a read OOM the runtime. Left unvalidated to
    /// match this crate's existing treatment of the
    /// `idle_ttl_secs`/`default_timeout_ms` relationship.
    #[serde(default = "default_scratch_mb")]
    pub scratch_mb: usize,
    /// Maximum number of files in that directory. Also the bound on the
    /// per-write scan, which is what keeps the derived quota cheap.
    #[serde(default = "default_scratch_files")]
    pub scratch_files: usize,
    /// Where scratch directories are created. `None` uses the system temp
    /// directory, which is tmpfs (host RAM) on most Linux hosts — point this
    /// at real disk on a memory-tight host. Deliberately not `TMPDIR`, which
    /// is process-global and shared with everything else the worker does.
    #[serde(default)]
    pub scratch_root: Option<String>,
}

pub fn default_max_runtimes() -> usize {
    32
}
pub fn default_default_timeout_ms() -> u64 {
    5_000
}
pub fn default_max_timeout_ms() -> u64 {
    30_000
}
pub fn default_heap_mb() -> usize {
    128
}
pub fn default_external_mb() -> usize {
    64
}
pub fn default_idle_ttl_secs() -> u64 {
    900
}
pub fn default_scratch_mb() -> usize {
    8
}
pub fn default_scratch_files() -> usize {
    64
}

impl Default for NodeEngineConfig {
    fn default() -> Self {
        Self {
            max_runtimes: default_max_runtimes(),
            default_timeout_ms: default_default_timeout_ms(),
            max_timeout_ms: default_max_timeout_ms(),
            heap_mb: default_heap_mb(),
            external_mb: default_external_mb(),
            idle_ttl_secs: default_idle_ttl_secs(),
            scratch_mb: default_scratch_mb(),
            scratch_files: default_scratch_files(),
            scratch_root: None,
        }
    }
}

impl NodeEngineConfig {
    /// Resolve a request's `timeout_ms`: absent falls back to the configured
    /// default, present is clamped to `max_timeout_ms`. Zero is treated as
    /// absent rather than as "expire immediately", which would make every
    /// call fail in a way that reads like a bug.
    pub fn clamp_timeout(&self, requested: Option<u64>) -> Duration {
        let ms = match requested {
            None | Some(0) => self.default_timeout_ms,
            Some(v) => v.min(self.max_timeout_ms),
        };
        Duration::from_millis(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Was `defaults_from_empty_yaml`. Driven through JSON here because
    /// `serde_yaml` is the worker's dependency, not this crate's — serde's
    /// field defaults are format-independent, so this asserts the same
    /// property. The YAML-shaped tests live beside `load_config`.
    #[test]
    fn defaults_from_empty_input() {
        let cfg: NodeEngineConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg, NodeEngineConfig::default());
    }

    #[test]
    fn clamp_timeout_uses_default_when_absent_or_zero() {
        let cfg = NodeEngineConfig::default();
        assert_eq!(cfg.clamp_timeout(None), Duration::from_millis(5_000));
        assert_eq!(cfg.clamp_timeout(Some(0)), Duration::from_millis(5_000));
    }

    #[test]
    fn clamp_timeout_caps_at_max() {
        let cfg = NodeEngineConfig::default();
        assert_eq!(cfg.clamp_timeout(Some(1_000)), Duration::from_millis(1_000));
        assert_eq!(
            cfg.clamp_timeout(Some(999_999)),
            Duration::from_millis(30_000)
        );
    }
}
