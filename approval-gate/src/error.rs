//! Error conventions: every error crossing the RPC boundary carries a stable
//! snake_case, worker-prefixed code in a `code: message` shape (see
//! tech-specs/2026-06-agentic/README.md § Error conventions).

use iii_sdk::errors::Error;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalError {
    /// The request shape is invalid beyond what serde can reject
    /// (empty ids, `/` in ids, malformed cursor).
    #[error("approval/invalid_payload: {0}")]
    InvalidPayload(String),

    /// A `state::*` call failed; the operation cannot proceed safely.
    #[error("approval/state_unavailable: {0}")]
    StateUnavailable(String),

    /// `harness::function::resolve` failed; the pending record is kept
    /// so the decision stays resolvable until turn/session cleanup.
    #[error("approval/harness_unavailable: {0}")]
    HarnessUnavailable(String),
}

impl ApprovalError {
    /// The stable error code (the part before `:` on the wire).
    pub fn code(&self) -> &'static str {
        match self {
            ApprovalError::InvalidPayload(_) => "approval/invalid_payload",
            ApprovalError::StateUnavailable(_) => "approval/state_unavailable",
            ApprovalError::HarnessUnavailable(_) => "approval/harness_unavailable",
        }
    }
}

impl From<ApprovalError> for Error {
    fn from(e: ApprovalError) -> Self {
        Error::Handler(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_code_colon_message() {
        let variants = vec![
            ApprovalError::InvalidPayload("m".into()),
            ApprovalError::StateUnavailable("m".into()),
            ApprovalError::HarnessUnavailable("m".into()),
        ];
        for v in variants {
            assert!(
                v.to_string().starts_with(&format!("{}: ", v.code())),
                "display `{v}` must start with code `{}`",
                v.code()
            );
        }
    }
}
