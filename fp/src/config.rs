//! Config for the fp worker, owned by the builtin `configuration` worker
//! (registered under id `fp`). One knob: whether the `fp::pipe` guidance is
//! injected into agent system prompts. Flip it in the console's config
//! dialog (or via `configuration::set`) — it hot-applies, no restart.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FpConfig {
    /// Append the `fp::pipe` usage guidance (~750 tokens) to every agent
    /// system prompt via the harness `pre-generate` hook. On by default;
    /// turn it off to shrink prompts (the harness's `# Granted functions`
    /// catalog still advertises the `fp::*` ids) or for discovery
    /// evaluations, where an advertised worker cannot be discovered — for
    /// those, store the off value BEFORE the worker's first boot (seed
    /// `./data/configuration/fp.yaml` or call `configuration::set` first),
    /// since boot binds the hook before any later flip could land.
    pub inject_guidance: bool,
}

impl Default for FpConfig {
    fn default() -> Self {
        Self {
            inject_guidance: true,
        }
    }
}

impl FpConfig {
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(FpConfig)).expect("FpConfig schema serializes")
    }

    /// Parse from the flat JSON object the configuration worker stores;
    /// missing keys fall back to defaults (`#[serde(default)]`).
    pub fn from_json(v: &serde_json::Value) -> Result<FpConfig, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("invalid fp config: {e}"))
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("FpConfig serializes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn injection_defaults_on() {
        assert!(FpConfig::default().inject_guidance);
        let parsed = FpConfig::from_json(&json!({})).unwrap();
        assert!(parsed.inject_guidance);
    }

    #[test]
    fn parses_the_stored_flat_shape() {
        // `false` is the non-default value, so this parse is discriminating.
        let flat = FpConfig::from_json(&json!({ "inject_guidance": false })).unwrap();
        assert!(!flat.inject_guidance);
    }

    #[test]
    fn round_trips_through_json() {
        let cfg = FpConfig {
            inject_guidance: false,
        };
        assert_eq!(FpConfig::from_json(&cfg.to_json()).unwrap(), cfg);
    }
}
