//! Coded failures (spec §12). Every function fails with `Error::Remote
//! {code, message}`, as `secrets/src/error.rs` does. No message carries a token
//! or a secret value: callers build messages from fixed text, ids and stderr
//! of local tools, never from a credential.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A failure with a stable code.
#[derive(Clone, PartialEq, Eq)]
pub struct CloudError {
    pub code: String,
    pub message: String,
    /// The cloud's HTTP status, for `CLOUD_ERROR`.
    pub status: Option<u16>,
    pub fix: Option<String>,
}

pub type Res<T> = Result<T, CloudError>;

impl CloudError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            status: None,
            fix: None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("INVALID_REQUEST", message)
    }

    pub fn not_connected() -> Self {
        Self::new(
            "NOT_CONNECTED",
            "Not signed in to iii cloud. Call iii-cloud::connect.",
        )
    }

    pub fn no_project() -> Self {
        Self::new(
            "NO_PROJECT",
            "No environment is selected. Call iii-cloud::project::use.",
        )
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_fix(mut self, fix: Option<String>) -> Self {
        self.fix = fix;
        self
    }

    /// The record form, for `Connection.error`, `Run.error` and friends.
    pub fn to_worker_error(&self) -> WorkerError {
        WorkerError {
            code: self.code.clone(),
            message: self.message.clone(),
            status: self.status,
            fix: self.fix.clone(),
        }
    }
}

/// `{code, message, status?, fix?}` inside records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl fmt::Display for CloudError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl fmt::Debug for CloudError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for CloudError {}

impl From<CloudError> for iii_sdk::errors::Error {
    fn from(error: CloudError) -> Self {
        let message = match (&error.fix, error.code.as_str()) {
            (Some(fix), "CLOUD_ERROR") => format!("{} Fix: {fix}", error.message),
            _ => error.message,
        };
        iii_sdk::errors::Error::Remote {
            code: error.code,
            message,
            stacktrace: None,
        }
    }
}

/// Cut text to `max` bytes on a char boundary (local tool stderr: 2 KB).
pub fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}
