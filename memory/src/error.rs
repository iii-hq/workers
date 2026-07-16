//! Worker error shape, mapped onto the bus error string as `code: message`.

use iii_sdk::errors::Error;

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("bank_not_found: {0}")]
    BankNotFound(String),

    #[error("memory_not_found: {0}")]
    MemoryNotFound(String),

    #[error("invalid_input: {0}")]
    InvalidInput(String),

    #[error("storage: {0}")]
    Storage(String),

    #[error("sibling_unavailable: {0}")]
    SiblingUnavailable(String),
}

impl From<MemoryError> for Error {
    fn from(e: MemoryError) -> Self {
        Error::Handler(e.to_string())
    }
}

impl From<std::io::Error> for MemoryError {
    fn from(e: std::io::Error) -> Self {
        MemoryError::Storage(e.to_string())
    }
}
