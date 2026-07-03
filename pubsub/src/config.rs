//! Worker configuration: which pub/sub backend the hub uses.
//! Mirrors the builtin's PubSubModuleConfig
//! (engine/src/workers/pubsub/config.rs) — one `adapter` field, closed set
//! `local` (default) / `redis`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_ADAPTER: &str = "local";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdapterEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PubSubConfig {
    /// Pub/sub backend selection and its adapter-specific config. `local`
    /// (default, in-process broadcast) or `redis` (cross-instance Redis
    /// Pub/Sub, config: `{redis_url}`). Hot-swap tier: a runtime edit rebuilds
    /// the backend, re-subscribes the live subscriptions onto it, and tears
    /// down the previous one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<AdapterEntry>,
}

impl PubSubConfig {
    pub fn effective_adapter_name(&self) -> &str {
        self.adapter
            .as_ref()
            .map(|a| a.name.as_str())
            .unwrap_or(DEFAULT_ADAPTER)
    }

    /// The `(name, config)` pair an adapter would be built from, normalizing
    /// an absent adapter to the default — parity with the builtin's
    /// `effective_adapter` (pubsub.rs:443-449) so `None` vs `Some(local)` is
    /// not a false change in the config hot-swap.
    pub fn effective_adapter(&self) -> (String, Option<Value>) {
        (
            self.effective_adapter_name().to_string(),
            self.adapter.as_ref().and_then(|a| a.config.clone()),
        )
    }

    pub fn normalized(&self) -> Self {
        self.clone()
    }

    pub fn json_schema() -> Value {
        serde_json::to_value(schemars::schema_for!(PubSubConfig)).unwrap_or(Value::Null)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone())
            .map_err(|e| format!("invalid pubsub configuration: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_local_adapter() {
        let c = PubSubConfig::default();
        assert_eq!(c.effective_adapter_name(), "local");
    }

    #[test]
    fn deserializes_adapter_entry() {
        let c: PubSubConfig = serde_yaml::from_str(
            "{adapter: {name: redis, config: {redis_url: 'redis://x:6379'}}}",
        )
        .unwrap();
        assert_eq!(c.effective_adapter_name(), "redis");
        assert_eq!(
            c.adapter.unwrap().config.unwrap()["redis_url"],
            "redis://x:6379"
        );
    }

    #[test]
    fn rejects_unknown_fields() {
        let r: Result<PubSubConfig, _> = serde_yaml::from_str("{adapter: {name: local}, wat: 1}");
        assert!(r.is_err(), "deny_unknown_fields must reject 'wat'");
    }

    #[test]
    fn effective_adapter_treats_none_as_default() {
        // Parity with the builtin's effective_adapter (pubsub.rs:443-449):
        // absent adapter and explicit `local` are NOT a change.
        let none = PubSubConfig::default();
        let local: PubSubConfig = serde_yaml::from_str("{adapter: {name: local}}").unwrap();
        let redis: PubSubConfig = serde_yaml::from_str("{adapter: {name: redis}}").unwrap();
        assert_eq!(none.effective_adapter(), local.effective_adapter());
        assert_ne!(none.effective_adapter(), redis.effective_adapter());
    }

    #[test]
    fn json_roundtrip() {
        let c: PubSubConfig = serde_yaml::from_str("{adapter: {name: redis}}").unwrap();
        let back = PubSubConfig::from_json(&c.to_json()).unwrap();
        assert_eq!(back.effective_adapter_name(), "redis");
    }
}
