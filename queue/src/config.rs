//! Worker configuration: queue transport plus durable named function queues.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::FunctionQueueConfig;

pub const DEFAULT_ADAPTER: &str = "builtin";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdapterEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueueConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<AdapterEntry>,
    /// Durable function queues keyed by the name used by
    /// `TriggerAction::Enqueue`. Definitions are persisted through the
    /// configuration worker and restored when this worker restarts.
    #[serde(default)]
    pub queue_configs: BTreeMap<String, FunctionQueueConfig>,
}

impl QueueConfig {
    pub fn effective_adapter_name(&self) -> &str {
        self.adapter
            .as_ref()
            .map(|a| a.name.as_str())
            .unwrap_or(DEFAULT_ADAPTER)
    }

    /// Registry/install seed. Direct library construction keeps the lightweight
    /// in-memory default, while installed workers persist accepted jobs.
    pub fn packaged_default() -> Self {
        Self {
            adapter: Some(AdapterEntry {
                name: DEFAULT_ADAPTER.to_string(),
                config: Some(serde_json::json!({
                    "store_method": "file_based",
                    "file_path": "./data/queue",
                    "save_interval_ms": 5000
                })),
            }),
            queue_configs: BTreeMap::new(),
        }
    }

    pub fn normalized(&self) -> Self {
        self.clone()
    }

    pub fn validate(&self) -> Result<(), String> {
        for (name, config) in &self.queue_configs {
            config.validate(name)?;
        }
        Ok(())
    }

    pub fn json_schema() -> Value {
        serde_json::to_value(schemars::schema_for!(QueueConfig)).unwrap_or(Value::Null)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        let config: Self = serde_json::from_value(value.clone())
            .map_err(|e| format!("invalid queue configuration: {e}"))?;
        config.validate()?;
        Ok(config)
    }
}

/// The active adapter's `config` object, Null-safe: an absent
/// `adapter`/`adapter.config`, and an explicit JSON `null`, both come back as
/// `None` rather than `Some(Value::Null)` -- adapter constructors
/// (`RedisAdapter::from_config`, `RabbitMQAdapter::from_config`, ...) only
/// ever need to distinguish "no config" from "an actual object".
pub fn adapter_config_value(config: &QueueConfig) -> Option<Value> {
    config
        .adapter
        .as_ref()
        .and_then(|adapter| adapter.config.clone())
        .filter(|value| !value.is_null())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_builtin_adapter() {
        let c = QueueConfig::default();
        assert_eq!(c.effective_adapter_name(), "builtin");
    }

    #[test]
    fn packaged_default_is_file_backed() {
        let config = QueueConfig::packaged_default();
        assert_eq!(config.effective_adapter_name(), "builtin");
        assert_eq!(
            config.adapter.unwrap().config.unwrap()["store_method"],
            "file_based"
        );
    }

    #[test]
    fn deserializes_adapter_entry() {
        let c: QueueConfig =
            serde_yaml::from_str("{adapter: {name: file_based, config: {path: './data/queue'}}}")
                .unwrap();
        assert_eq!(c.effective_adapter_name(), "file_based");
        assert_eq!(c.adapter.unwrap().config.unwrap()["path"], "./data/queue");
    }

    #[test]
    fn rejects_unknown_fields() {
        let r: Result<QueueConfig, _> = serde_yaml::from_str("{adapter: {name: builtin}, wat: 1}");
        assert!(r.is_err(), "deny_unknown_fields must reject 'wat'");
    }

    #[test]
    fn json_roundtrip() {
        let c: QueueConfig = serde_yaml::from_str("{adapter: {name: file_based}}").unwrap();
        let back = QueueConfig::from_json(&c.to_json()).unwrap();
        assert_eq!(back.effective_adapter_name(), "file_based");
    }

    #[test]
    fn adapter_config_value_none_without_adapter() {
        assert_eq!(adapter_config_value(&QueueConfig::default()), None);
    }

    #[test]
    fn adapter_config_value_none_without_config() {
        let c = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "redis".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };
        assert_eq!(adapter_config_value(&c), None);
    }

    #[test]
    fn adapter_config_value_none_for_explicit_null() {
        let c = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "redis".to_string(),
                config: Some(serde_json::Value::Null),
            }),
            ..QueueConfig::default()
        };
        assert_eq!(adapter_config_value(&c), None);
    }

    #[test]
    fn adapter_config_value_some_for_object() {
        let c = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "redis".to_string(),
                config: Some(serde_json::json!({"redis_url": "redis://x"})),
            }),
            ..QueueConfig::default()
        };
        assert_eq!(
            adapter_config_value(&c),
            Some(serde_json::json!({"redis_url": "redis://x"}))
        );
    }

    #[test]
    fn named_queue_config_roundtrips() {
        let c: QueueConfig = serde_yaml::from_str(
            "queue_configs:\n  harness-turn:\n    type: fifo\n    message_group_field: session_id\n",
        )
        .unwrap();
        c.validate().unwrap();
        assert_eq!(
            c.queue_configs["harness-turn"]
                .message_group_field
                .as_deref(),
            Some("session_id")
        );
        assert_eq!(QueueConfig::from_json(&c.to_json()).unwrap(), c);
    }
}
