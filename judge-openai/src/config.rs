//! Operator-owned credentials and default model. No caller-controlled endpoint.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct OpenAiConfig {
    /// Provider credential, or a secret://NAME reference (e.g. secret://OPENAI_API_KEY)
    /// resolved through the secrets worker at call time. A nonblank value overrides
    /// OPENAI_API_KEY captured at boot; a reference that does not resolve fails with
    /// missing_key instead of falling back to it.
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
impl std::fmt::Debug for OpenAiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiConfig")
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("model", &self.model)
            .field("max_request_bytes", &self.max_request_bytes)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("max_timeout_ms", &self.max_timeout_ms)
            .finish()
    }
}
impl Default for OpenAiConfig {
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
impl OpenAiConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !crate::SUPPORTED_MODELS.contains(&self.model.as_str()) {
            return Err("OpenAI model is not supported by the Decisions API".into());
        }
        self.execution_limits()
            .validate()
            .map_err(|_| "Invalid OpenAI execution limits".to_string())?;
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
            .map_err(|_| "Invalid OpenAI configuration".to_string())?;
        config.validate()?;
        Ok(config)
    }
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("OpenAI config serializes")
    }
    pub fn json_schema() -> Value {
        let mut schema =
            serde_json::to_value(schemars::schema_for!(Self)).expect("OpenAI schema serializes");
        schema["properties"]["api_key"]["format"] = "password".into();
        schema["properties"]["api_key"]["writeOnly"] = true.into();
        schema["properties"]["model"]["enum"] = crate::SUPPORTED_MODELS.into();
        schema
    }
}
