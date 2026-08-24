#[derive(Debug, thiserror::Error)]
pub enum SecurityScanError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("dependency failure: {0}")]
    Dependency(String),
}

impl From<SecurityScanError> for iii_sdk::errors::Error {
    fn from(error: SecurityScanError) -> Self {
        Self::Handler(error.to_string())
    }
}
