//! The worker's structured error taxonomy. Carries no capability and no
//! tenant source code — only the message/traceback a tenant's own execution
//! produced — so unlike `node-engine::error`, this can `#[derive(Debug)]`
//! freely instead of hand-rolling a redacting one.
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidInput,
    SyntaxError,
    PythonException,
    Timeout,
    OutOfMemory,
    ResultTooLarge,
    DiskQuotaExceeded,
    /// The `runtime_id` names nothing live — never created, already torn
    /// down, or reaped for idleness.
    RuntimeNotFound,
    /// `max_runtimes` live runtimes already exist. An ADMISSION failure:
    /// retrying later may succeed, unlike the mid-run caps above.
    Capacity,
    Internal,
}

impl ErrorKind {
    /// The bare snake_case wire code — the same spelling `#[serde(rename_all
    /// = "snake_case")]` produces, but without the JSON string quotes, since
    /// this feeds a `code` field directly rather than a JSON document.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::SyntaxError => "syntax_error",
            Self::PythonException => "python_exception",
            Self::Timeout => "timeout",
            Self::OutOfMemory => "out_of_memory",
            Self::ResultTooLarge => "result_too_large",
            Self::DiskQuotaExceeded => "disk_quota_exceeded",
            Self::RuntimeNotFound => "runtime_not_found",
            Self::Capacity => "capacity",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PythonEngineError {
    pub kind: ErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traceback: Option<String>,
}

impl PythonEngineError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            traceback: None,
        }
    }
}

impl std::fmt::Display for PythonEngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.message)
    }
}
impl std::error::Error for PythonEngineError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_serialize_snake_case() {
        let e = PythonEngineError::new(ErrorKind::ResultTooLarge, "too big");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "result_too_large");
        assert!(v.get("traceback").is_none());
    }

    /// `ErrorKind::as_str` is hand-written (it feeds a bare wire `code`
    /// field, so it can't reuse the JSON-quoted serde output) — pin it
    /// against the serde spelling for every variant so the two can't drift.
    #[test]
    fn as_str_agrees_with_serde_rename_for_every_variant() {
        for kind in [
            ErrorKind::InvalidInput,
            ErrorKind::SyntaxError,
            ErrorKind::PythonException,
            ErrorKind::Timeout,
            ErrorKind::OutOfMemory,
            ErrorKind::ResultTooLarge,
            ErrorKind::DiskQuotaExceeded,
            ErrorKind::RuntimeNotFound,
            ErrorKind::Capacity,
            ErrorKind::Internal,
        ] {
            let serde_spelling = serde_json::to_value(kind).unwrap();
            assert_eq!(serde_spelling, kind.as_str(), "drift for {kind:?}");
        }
    }
}
