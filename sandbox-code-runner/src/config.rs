use std::time::Duration;

use anyhow::Result;
use serde::Deserialize;

/// Operator-facing limits. Every field has a `serde(default)` so an empty or
/// partial `config.yaml` still yields a fully-populated struct. Image
/// allowlists, CPU/memory caps, and sandbox concurrency are the iii-sandbox
/// daemon's config, deliberately not duplicated here.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CodeRunnerConfig {
    #[serde(default = "default_default_timeout_ms")]
    pub default_timeout_ms: u64,
    #[serde(default = "default_max_timeout_ms")]
    pub max_timeout_ms: u64,
    /// Passed to `sandbox::create` as `idle_timeout_secs`. The daemon's idle
    /// reaper is the ONLY reaper — sandbox-code-runner runs no sweep of its
    /// own; a reaped VM surfaces as `sandbox-code-runner::expired` on the
    /// next call.
    #[serde(default = "default_idle_ttl_secs")]
    pub idle_ttl_secs: u64,
}

fn default_default_timeout_ms() -> u64 {
    5_000
}
fn default_max_timeout_ms() -> u64 {
    30_000
}
fn default_idle_ttl_secs() -> u64 {
    900
}

/// Floor for `effective_idle_ttl_secs`. The daemon itself enforces no
/// minimum on `idle_timeout_secs`, so an operator-set 0 or 1 would make
/// every runtime reapable almost as soon as it's created — in the worst
/// case, before the runner plant's own `sandbox::fs::write` lands right
/// after `sandbox::create` returns (see `RuntimeManager::create`'s
/// `map_failure` doc). `FS_TIMEOUT_MS`'s scale (30s) is the daemon-side
/// deadline that same plant call gets; a runtime that can't outlive its own
/// creation sequence under normal conditions is a config bug, not a choice.
const MIN_IDLE_TTL_SECS: u64 = 30;

impl Default for CodeRunnerConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: default_default_timeout_ms(),
            max_timeout_ms: default_max_timeout_ms(),
            idle_ttl_secs: default_idle_ttl_secs(),
        }
    }
}

impl CodeRunnerConfig {
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

    /// The `idle_timeout_secs` value actually sent to `sandbox::create`:
    /// `idle_ttl_secs` floored at `MIN_IDLE_TTL_SECS`. Floors, not the raw
    /// field — `config.yaml` round-trip tests below check the field as
    /// written; only the value that reaches the daemon is protected.
    pub fn effective_idle_ttl_secs(&self) -> u64 {
        self.idle_ttl_secs.max(MIN_IDLE_TTL_SECS)
    }
}

pub fn load_config(path: &str) -> Result<CodeRunnerConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: CodeRunnerConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: CodeRunnerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg, CodeRunnerConfig::default());
    }

    #[test]
    fn custom_yaml_overrides_each_field() {
        let cfg: CodeRunnerConfig = serde_yaml::from_str(
            "default_timeout_ms: 100\nmax_timeout_ms: 200\nidle_ttl_secs: 5\n",
        )
        .unwrap();
        assert_eq!(
            cfg,
            CodeRunnerConfig {
                default_timeout_ms: 100,
                max_timeout_ms: 200,
                idle_ttl_secs: 5,
            }
        );
    }

    #[test]
    fn partial_yaml_keeps_other_defaults() {
        let cfg: CodeRunnerConfig = serde_yaml::from_str("idle_ttl_secs: 60").unwrap();
        assert_eq!(cfg.idle_ttl_secs, 60);
        assert_eq!(cfg.default_timeout_ms, default_default_timeout_ms());
    }

    #[test]
    fn clamp_timeout_uses_default_when_absent_or_zero() {
        let cfg = CodeRunnerConfig::default();
        assert_eq!(cfg.clamp_timeout(None), Duration::from_millis(5_000));
        assert_eq!(cfg.clamp_timeout(Some(0)), Duration::from_millis(5_000));
    }

    #[test]
    fn clamp_timeout_caps_at_max() {
        let cfg = CodeRunnerConfig::default();
        assert_eq!(cfg.clamp_timeout(Some(1_000)), Duration::from_millis(1_000));
        assert_eq!(
            cfg.clamp_timeout(Some(999_999)),
            Duration::from_millis(30_000)
        );
    }

    /// Residual finding (final review, second pass): the daemon enforces no
    /// floor on `idle_timeout_secs`, so an operator-set 0 or 1 would make a
    /// runtime reapable almost immediately — in the worst case, before the
    /// runner plant's own `fs::write` lands right after `sandbox::create`
    /// returns. Only the EFFECTIVE value is floored; the raw field is not
    /// mutated, so config.yaml round-trips (below) still see what was
    /// actually written.
    #[test]
    fn effective_idle_ttl_secs_has_a_floor() {
        let below_floor = CodeRunnerConfig {
            idle_ttl_secs: 0,
            ..CodeRunnerConfig::default()
        };
        assert_eq!(below_floor.effective_idle_ttl_secs(), 30);

        let just_one = CodeRunnerConfig {
            idle_ttl_secs: 1,
            ..CodeRunnerConfig::default()
        };
        assert_eq!(just_one.effective_idle_ttl_secs(), 30);

        let above_floor = CodeRunnerConfig {
            idle_ttl_secs: 60,
            ..CodeRunnerConfig::default()
        };
        assert_eq!(
            above_floor.effective_idle_ttl_secs(),
            60,
            "above the floor, unchanged"
        );
        assert_eq!(
            above_floor.idle_ttl_secs, 60,
            "the raw field is never mutated"
        );
    }

    #[test]
    fn committed_config_yaml_matches_struct_defaults() {
        let cfg = load_config("config.yaml").expect("committed config.yaml parses");
        assert_eq!(cfg, CodeRunnerConfig::default());
    }
}
