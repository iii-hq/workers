//! Error conventions: every error crossing the bus carries a stable
//! snake_case, worker-prefixed code in a `code: message` shape (README
//! § Error conventions). The spec names `harness/invalid_message_role`,
//! `harness/spawn_depth_exceeded`, and `harness/spawn_fanout_exceeded`
//! verbatim.

use iii_sdk::errors::Error;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HarnessError {
    /// A `harness::send` message whose role is not `user` or `custom`.
    #[error("harness/invalid_message_role: {0}")]
    InvalidMessageRole(String),

    /// Request shape invalid beyond what serde rejects.
    #[error("harness/invalid_request: {0}")]
    InvalidRequest(String),

    /// `harness::spawn` past the depth budget.
    #[error("harness/spawn_depth_exceeded: {0}")]
    SpawnDepthExceeded(String),

    /// `harness::spawn` past the fan-out budget.
    #[error("harness/spawn_fanout_exceeded: {0}")]
    SpawnFanoutExceeded(String),

    /// A required dependency call failed (session-manager, llm-router).
    #[error("harness/dependency: {0}")]
    Dependency(String),

    /// No provider-safe context can fit inside the resolved model budget.
    #[error("harness/context_overflow: {0}")]
    ContextOverflow(String),

    /// A state read/write backing the loop failed.
    #[error("harness/state: {0}")]
    State(String),

    /// An unexpected internal failure inside a loop step.
    #[error("harness/internal: {0}")]
    Internal(String),
}

impl HarnessError {
    /// The stable error code (the part before `:` on the wire).
    pub fn code(&self) -> &'static str {
        match self {
            HarnessError::InvalidMessageRole(_) => "harness/invalid_message_role",
            HarnessError::InvalidRequest(_) => "harness/invalid_request",
            HarnessError::SpawnDepthExceeded(_) => "harness/spawn_depth_exceeded",
            HarnessError::SpawnFanoutExceeded(_) => "harness/spawn_fanout_exceeded",
            HarnessError::Dependency(_) => "harness/dependency",
            HarnessError::ContextOverflow(_) => "harness/context_overflow",
            HarnessError::State(_) => "harness/state",
            HarnessError::Internal(_) => "harness/internal",
        }
    }
}

impl From<HarnessError> for Error {
    fn from(e: HarnessError) -> Self {
        Error::Handler(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_display_starts_with_its_code() {
        let variants = vec![
            HarnessError::InvalidMessageRole("m".into()),
            HarnessError::InvalidRequest("m".into()),
            HarnessError::SpawnDepthExceeded("m".into()),
            HarnessError::SpawnFanoutExceeded("m".into()),
            HarnessError::Dependency("m".into()),
            HarnessError::ContextOverflow("m".into()),
            HarnessError::State("m".into()),
            HarnessError::Internal("m".into()),
        ];
        for v in variants {
            assert!(
                v.to_string().starts_with(&format!("{}: ", v.code())),
                "display `{v}` must start with code `{}`",
                v.code()
            );
        }
    }

    #[test]
    fn context_overflow_has_stable_code() {
        let error = HarnessError::ContextOverflow("request does not fit".into());
        assert_eq!(error.code(), "harness/context_overflow");
        assert_eq!(
            error.to_string(),
            "harness/context_overflow: request does not fit"
        );
    }
}
