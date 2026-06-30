use iii_sdk::errors::Error;

#[derive(Debug, thiserror::Error)]
pub enum WorkflowError {
    #[error("invalid workflow definition: {0}")]
    InvalidDef(String),
    #[error("state error: {0}")]
    State(String),
    #[error("trigger error: {0}")]
    Trigger(String),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

impl From<WorkflowError> for Error {
    fn from(e: WorkflowError) -> Self {
        Error::Handler(e.to_string())
    }
}
