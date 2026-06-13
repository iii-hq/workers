//! Operator-facing runtime configuration loaded from `config.yaml`.
//!
//! Every field has a serde default so `{}` (and a missing file) yields a
//! fully-populated config; per-call request options override these
//! defaults where the spec allows it (reserved tokens, tail turns, prune
//! thresholds).

use anyhow::Result;
use serde::Deserialize;

/// Root config shape. Unknown keys are rejected so a typo'd field
/// (e.g. `tail_truns: 3`) fails loudly instead of silently running the
/// default.
#[derive(Deserialize, Debug, Clone, PartialEq)]
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
    }

    #[test]
    fn custom_yaml_overrides_every_field() {
        let yaml = "reserved_tokens_cap: 1\nreserved_pct: 2\ntail_turns: 3\n\
                    protect_recent_tokens: 4\nmin_free_tokens: 5\nmax_output_chars: 6\n\
                    lease_ttl_secs: 7\nallow_fallback_limits: false\nsummarizer_timeout_ms: 8";
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
}
