//! Worker configuration: which lock backend the scheduler uses.
//! Mirrors the builtin's CronModuleConfig (engine/src/workers/cron/config.rs)
//! with `kv` renamed to `local` (the builtin kv lock was process-local too).

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
pub struct CronConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<AdapterEntry>,
}

impl CronConfig {
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
        serde_json::to_value(schemars::schema_for!(CronConfig)).unwrap_or(Value::Null)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone())
            .map_err(|e| format!("invalid cron configuration: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_local_adapter() {
        let c = CronConfig::default();
        assert_eq!(c.effective_adapter_name(), "local");
    }

    #[test]
    fn deserializes_adapter_entry() {
        let c: CronConfig =
            serde_yaml::from_str("{adapter: {name: redis, config: {redis_url: 'redis://x:6379'}}}")
                .unwrap();
        assert_eq!(c.effective_adapter_name(), "redis");
        assert_eq!(
            c.adapter.unwrap().config.unwrap()["redis_url"],
            "redis://x:6379"
        );
    }

    #[test]
    fn rejects_unknown_fields() {
        let r: Result<CronConfig, _> = serde_yaml::from_str("{adapter: {name: local}, wat: 1}");
        assert!(r.is_err(), "deny_unknown_fields must reject 'wat'");
    }

    #[test]
    fn json_roundtrip() {
        let c: CronConfig = serde_yaml::from_str("{adapter: {name: redis}}").unwrap();
        let back = CronConfig::from_json(&c.to_json()).unwrap();
        assert_eq!(back.effective_adapter_name(), "redis");
    }
}
