//! Worker configuration: which queue transport adapter this worker uses.
//! Mirrors the builtin's QueueModuleConfig (engine/src/workers/queue/config.rs)
//! with the default in-process transport named `builtin`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}

impl QueueConfig {
    pub fn effective_adapter_name(&self) -> &str {
        self.adapter
            .as_ref()
            .map(|a| a.name.as_str())
            .unwrap_or(DEFAULT_ADAPTER)
    }

    pub fn normalized(&self) -> Self {
        self.clone()
    }

    pub fn json_schema() -> Value {
        serde_json::to_value(schemars::schema_for!(QueueConfig)).unwrap_or(Value::Null)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone())
            .map_err(|e| format!("invalid queue configuration: {e}"))
    }
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
}
