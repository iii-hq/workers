use iii_sdk::errors::Error as SdkError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvalPhaseV1 {
    Setup,
    Send,
    Await,
    Collect,
    Evaluate,
    Limit,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalFailureV1 {
    pub phase: EvalPhaseV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    pub message: String,
}

impl EvalFailureV1 {
    pub fn new(phase: EvalPhaseV1, message: impl Into<String>) -> Self {
        Self {
            phase,
            code: None,
            function_id: None,
            message: message.into(),
        }
    }

    pub fn function(
        phase: EvalPhaseV1,
        function_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            phase,
            code: None,
            function_id: Some(function_id.into()),
            message: message.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("evaluation not found: {0}")]
    NotFound(String),
    #[error("evaluation conflict: {0}")]
    Conflict(String),
    #[error("dependency error: {0}")]
    Dependency(String),
    #[error("state error: {0}")]
    State(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<serde_json::Error> for EvalError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error.to_string())
    }
}

impl From<EvalError> for SdkError {
    fn from(error: EvalError) -> Self {
        SdkError::Handler(error.to_string())
    }
}
