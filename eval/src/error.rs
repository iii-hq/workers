use iii_sdk::errors::Error as SdkError;

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("evaluation not found: {0}")]
    NotFound(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
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
