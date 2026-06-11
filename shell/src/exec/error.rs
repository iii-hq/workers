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
    ///
    /// The handler-return path lifts `ExecError` to `IIIError::Remote` directly
    /// (see `From<ExecError> for IIIError` below), so it no longer stringifies.
    /// `to_json` is kept as the canonical `{code,message}` serialization
    /// (round-trip coverage in tests) and for any caller that needs the wire
    /// shape as a `String`.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ExecError is always serializable")
    }
}

/// Carry the S-code to the wire as the top-level `code`. The engine SDK maps
/// `IIIError::Remote { code, message, .. }` to the wire `ErrorBody` verbatim,
/// so an agent can branch on `error.code` (e.g. "S211"). Any other `IIIError`
/// variant collapses to `code: "invocation_failed"` with the real code buried
/// in the message — which is exactly what we are escaping here.
impl From<ExecError> for iii_sdk::IIIError {
    fn from(err: ExecError) -> Self {
        iii_sdk::IIIError::Remote {
            code: err.code.to_string(),
            message: err.message,
            stacktrace: None,
        }
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

    /// The wire contract: `ExecError` lifts to `IIIError::Remote { code, .. }`
    /// so the S-code reaches the wire `code` verbatim. Any other variant (e.g.
    /// Handler) would collapse to `code: "invocation_failed"` — pin against that
    /// regression so an agent can keep branching on `error.code`.
    #[test]
    fn converts_to_iii_remote_carrying_the_s_code() {
        let err: iii_sdk::IIIError = ExecError::new("S216", "host exec: boom").into();
        match err {
            iii_sdk::IIIError::Remote {
                code,
                message,
                stacktrace,
            } => {
                assert_eq!(code, "S216");
                assert_eq!(message, "host exec: boom");
                assert!(stacktrace.is_none());
            }
            other => panic!("expected IIIError::Remote, got {other:?}"),
        }
    }
}
