//! Operator-owned credentials and default model. No caller-controlled endpoint.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct JevConfig {
    /// Provider credential. A nonblank value overrides TYPESAFE_API_KEY captured at boot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Default model for requests that omit their model.
    pub model: String,
    /// Maximum encoded body bytes for each provider evaluation.
    #[schemars(range(min = 1))]
    pub max_request_bytes: usize,
    /// Maximum response bytes, including streamed bodies.
    #[schemars(range(min = 1))]
    pub max_response_bytes: usize,
    /// Maximum relative deadline for evaluations and model listing.
    #[schemars(range(min = 1))]
    pub max_timeout_ms: u64,
}
impl std::fmt::Debug for JevConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JevConfig")
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("model", &self.model)
            .field("max_request_bytes", &self.max_request_bytes)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("max_timeout_ms", &self.max_timeout_ms)
            .finish()
    }
}
impl Default for JevConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model: crate::DEFAULT_MODEL.into(),
            max_request_bytes: judge_contract::DEFAULT_MAX_REQUEST_BYTES,
            max_response_bytes: judge_contract::DEFAULT_MAX_RESPONSE_BYTES,
            max_timeout_ms: judge_contract::DEFAULT_MAX_TIMEOUT_MS,
        }
    }
}
impl JevConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model.trim().is_empty() {
            return Err("JEV model must not be blank".into());
        }
        self.execution_limits()
            .validate()
            .map_err(|_| "Invalid JEV execution limits".to_string())?;
        Ok(())
    }
    pub fn execution_limits(&self) -> crate::ExecutionLimits {
        crate::ExecutionLimits {
            max_request_bytes: self.max_request_bytes,
            max_response_bytes: self.max_response_bytes,
            max_timeout_ms: self.max_timeout_ms,
        }
    }
    /// Parse without including source values in errors (they may be credentials).
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let config: Self = serde_json::from_value(value.clone())
            .map_err(|_| "Invalid JEV configuration".to_string())?;
        config.validate()?;
        Ok(config)
    }
    /// Read an optional YAML or JSON seed; runtime authoritative values arrive
    /// through the configuration worker, which owns environment expansion.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|_| "Cannot read JEV configuration seed".to_string())?;
        let value: Value = serde_yaml::from_str(&text)
            .map_err(|_| "Invalid JEV configuration seed".to_string())?;
        Self::from_json(&value)
    }
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("JEV config serializes")
    }
    pub fn json_schema() -> Value {
        let mut schema =
            serde_json::to_value(schemars::schema_for!(Self)).expect("JEV schema serializes");
        schema["properties"]["api_key"]["format"] = "password".into();
        schema["properties"]["api_key"]["writeOnly"] = true.into();
        schema["properties"]["model"]["pattern"] = "\\S".into();
        schema
    }
}
