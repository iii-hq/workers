//! `ExecError` carries an S-code + message and serializes as
//! `{ "code": "...", "message": "..." }`, matching the engine daemon
//! and `fs::error::FsError`. Both shell-side and sandbox-forwarded
//! errors round-trip through the same shape so callers don't branch
//! on backend.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExecError {
    pub code: &'static str,
    pub message: String,
}

impl ExecError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Serializing `&'static str` + `String` is effectively infallible
    /// (OOM only); `expect` so future shape changes fail loudly rather
    /// than producing malformed JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ExecError is always serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_json_emits_code_and_message() {
        let e = ExecError::new("S300", "VM boot failed");
        let j = e.to_json();
        assert!(j.contains("\"code\":\"S300\""));
        assert!(j.contains("\"message\":\"VM boot failed\""));
    }

    #[test]
    fn equality_compares_code_and_message() {
        assert_eq!(ExecError::new("S210", "x"), ExecError::new("S210", "x"),);
        assert_ne!(ExecError::new("S210", "x"), ExecError::new("S211", "x"),);
    }
}
