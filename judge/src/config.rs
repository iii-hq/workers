//! Operator-owned default provider; requests may still name their own.
use crate::register::{validate_provider, DEFAULT_PROVIDER};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct JudgeConfig {
    /// `judge-<provider>` worker used when a request omits `provider`.
    pub provider: String,
    /// Keep every local provider's model loaded (judge-decider, judge-semif,
    /// judge-laya), not only the default provider's, so no request naming one
    /// waits for its load. Off, any other local provider loads its model on
    /// first use and releases it after 10 idle minutes. Hosted providers are
    /// unaffected. Off is not stored.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub preload_all: bool,
}
impl Default for JudgeConfig {
    fn default() -> Self {
        Self {
            provider: DEFAULT_PROVIDER.into(),
            preload_all: false,
        }
    }
}
impl JudgeConfig {
    pub fn validate(&self) -> Result<(), String> {
        validate_provider(&self.provider).map_err(|_| {
            "Judge provider must be a worker-name suffix: lowercase letters, digits and hyphens, at most 64 bytes".to_string()
        })
    }
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let config: Self = serde_json::from_value(value.clone())
            .map_err(|_| "Invalid judge configuration".to_string())?;
        config.validate()?;
        Ok(config)
    }
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("judge config serializes")
    }
    pub fn json_schema() -> Value {
        let mut schema =
            serde_json::to_value(schemars::schema_for!(Self)).expect("judge schema serializes");
        schema["properties"]["provider"]["pattern"] = "^[a-z0-9-]{1,64}$".into();
        schema
    }
}
