//! Typed failures. Every message is safe to log and to return over the bus:
//! it names a code and, at most, a validated secret name or a filesystem path.
//! No message ever carries a value, key material or unvalidated caller input
//! (a raw key passed where a name was expected must not be echoed back).
use std::fmt;

/// Wire error codes (`Error::Remote.code`).
pub mod codes {
    /// The payload does not match the function's request schema.
    pub const INVALID_REQUEST: &str = "INVALID_REQUEST";
    /// `secrets::resolve` got something that is neither `secret://NAME` nor a bare NAME.
    pub const INVALID_REFERENCE: &str = "INVALID_REFERENCE";
    pub const SECRET_NOT_FOUND: &str = "SECRET_NOT_FOUND";
    /// The calling worker is not in the secret's `consumers`.
    pub const SECRET_FORBIDDEN: &str = "SECRET_FORBIDDEN";
    /// `secrets::import`: the named source has no non-empty value for the name.
    pub const SOURCE_NOT_FOUND: &str = "SOURCE_NOT_FOUND";
    /// The master key is missing, unreadable or malformed.
    pub const KEY_UNAVAILABLE: &str = "KEY_UNAVAILABLE";
    /// A master key was found but it is not the one this vault was sealed with.
    pub const KEY_MISMATCH: &str = "KEY_MISMATCH";
    /// The vault file cannot be read, parsed or written.
    pub const VAULT_ERROR: &str = "VAULT_ERROR";
    /// A stored ciphertext failed authentication (tampered, or moved between names).
    pub const DECRYPT_FAILED: &str = "DECRYPT_FAILED";
    /// `engine::workers::list` failed, so the caller's identity is unknown.
    pub const CALLER_LOOKUP_FAILED: &str = "CALLER_LOOKUP_FAILED";
}

#[derive(Clone, PartialEq, Eq)]
pub struct SecretsError {
    pub code: &'static str,
    pub message: String,
}

impl SecretsError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(codes::INVALID_REQUEST, message)
    }

    pub fn not_found(name: &str) -> Self {
        Self::new(
            codes::SECRET_NOT_FOUND,
            format!("secret `{name}` is not stored"),
        )
    }

    pub fn vault(message: impl Into<String>) -> Self {
        Self::new(codes::VAULT_ERROR, message)
    }

    pub fn key(message: impl Into<String>) -> Self {
        Self::new(codes::KEY_UNAVAILABLE, message)
    }
}

impl fmt::Display for SecretsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl fmt::Debug for SecretsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for SecretsError {}

impl From<SecretsError> for iii_sdk::errors::Error {
    /// `Remote` keeps the code on the wire (`ErrorBody.code`) instead of the
    /// SDK's generic `invocation_failed`, and carries no backtrace.
    fn from(error: SecretsError) -> Self {
        iii_sdk::errors::Error::Remote {
            code: error.code.to_string(),
            message: error.message,
            stacktrace: None,
        }
    }
}
