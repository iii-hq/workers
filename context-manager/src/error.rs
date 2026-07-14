//! Error conventions: every error crossing the bus carries a stable
//! snake_case, worker-prefixed code in a `code: message` shape (see
//! tech-specs/2026-06-agentic/README.md § Error conventions).
//!
//! The two messages the spec names verbatim — `messages is required` and
//! `could not resolve model limits` — are kept word-for-word in the
//! message part so callers can match on the documented strings.

use iii_sdk::errors::Error;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContextError {
    /// The request shape is invalid beyond what serde can reject
    /// (e.g. `messages` absent: the spec's `messages is required`).
    #[error("context/invalid_request: {0}")]
    InvalidRequest(String),

    /// Neither inline limits nor `llm-router` could provide model limits
    /// and the conservative fallback is disabled.
    #[error("context/model_unresolved: could not resolve model limits ({0})")]
    ModelUnresolved(String),

    /// A filesystem operation backing the compaction lease failed.
    #[error("context/state: {0}")]
    State(String),

    /// No safe model-facing context can fit within the usable input
    /// budget after normal pruning, compaction, and emergency reduction.
    #[error(
        "context/overflow: assembled context requires {token_count} tokens but usable budget is {usable}"
    )]
    Overflow { token_count: u64, usable: u64 },
}

impl ContextError {
    /// The stable error code (the part before `:` on the wire).
    pub fn code(&self) -> &'static str {
        match self {
            ContextError::InvalidRequest(_) => "context/invalid_request",
            ContextError::ModelUnresolved(_) => "context/model_unresolved",
            ContextError::State(_) => "context/state",
            ContextError::Overflow { .. } => "context/overflow",
        }
    }
}

impl From<ContextError> for Error {
    fn from(e: ContextError) -> Self {
        Error::Handler(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_code_colon_message() {
        let e = ContextError::InvalidRequest("messages is required".into());
        assert_eq!(
            e.to_string(),
            "context/invalid_request: messages is required"
        );
        assert_eq!(e.code(), "context/invalid_request");
    }

    #[test]
    fn model_unresolved_keeps_the_spec_string() {
        let e = ContextError::ModelUnresolved("no inline limits, router absent".into());
        assert!(e.to_string().contains("could not resolve model limits"));
    }

    #[test]
    fn every_variant_display_starts_with_its_code() {
        let variants = vec![
            ContextError::InvalidRequest("m".into()),
            ContextError::ModelUnresolved("m".into()),
            ContextError::State("m".into()),
            ContextError::Overflow {
                token_count: 101,
                usable: 100,
            },
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
    fn overflow_carries_budget_values_and_stable_code() {
        let error = ContextError::Overflow {
            token_count: 12_345,
            usable: 10_000,
        };

        assert_eq!(error.code(), "context/overflow");
        assert_eq!(
            error.to_string(),
            "context/overflow: assembled context requires 12345 tokens but usable budget is 10000"
        );
        assert!(matches!(
            error,
            ContextError::Overflow {
                token_count: 12_345,
                usable: 10_000
            }
        ));
    }
}
