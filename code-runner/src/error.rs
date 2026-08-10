//! The worker's error taxonomy. Every variant maps to a stable
//! `code-runner::<snake_case>` wire code, matching sandbox-code-runner's
//! shape so a caller written against that worker needs no changes.
//!
//! One code is new: `resource_exhausted`. See its variant.

use iii_sdk::errors::Error;

/// Deliberate exception to the redaction convention: this type `Display`s
/// runtime ids to the holder — the caller who already supplied them. Every
/// other surface treats a `runtime_id` as a capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeRunnerError {
    InvalidRequest(String),
    RuntimeNotFound(String),
    /// `teardown namespace=…` naming a namespace with no runtime behind it.
    /// Same wire code as `RuntimeNotFound` (both mean "nothing addressable by
    /// this"), a distinct variant only because the wording differs.
    NamespaceNotFound(String),
    /// The runtime is gone — reaped for idleness, or killed mid-call.
    /// Re-create it.
    Expired(String),
    /// No runtime slot available. Admission failure: retrying later may work.
    Capacity(String),
    Timeout,
    /// A registered handler threw, or returned something JSON cannot carry.
    HandlerError(String),
    /// A cap was hit mid-run — memory, or the scratch quota.
    ///
    /// NOT `capacity`, which is documented as an admission failure and means
    /// "retry later, it may succeed". A mid-run resource kill fails identically
    /// on every retry, so folding the two together would tell a caller the
    /// opposite of the truth. One code for all such caps rather than three,
    /// because the caller's action is the same in every case — shrink the
    /// workload — and the message names which cap was hit.
    ResourceExhausted(String),
    Engine(String),
}

impl CodeRunnerError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "code-runner::invalid_request",
            Self::RuntimeNotFound(_) | Self::NamespaceNotFound(_) => {
                "code-runner::runtime_not_found"
            }
            Self::Expired(_) => "code-runner::expired",
            Self::Capacity(_) => "code-runner::capacity",
            Self::Timeout => "code-runner::timeout",
            Self::HandlerError(_) => "code-runner::handler_error",
            Self::ResourceExhausted(_) => "code-runner::resource_exhausted",
            Self::Engine(_) => "code-runner::engine",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::InvalidRequest(m) => m.clone(),
            Self::RuntimeNotFound(id) => format!("unknown runtime_id {id}"),
            Self::NamespaceNotFound(ns) => format!(
                "no runtime is registered for namespace {ns:?}; register a function in it \
                 first, or pass a runtime_id instead"
            ),
            Self::Expired(id) => format!(
                "runtime {id} expired: it was reaped or killed and its functions \
                 unregistered — call run again without this runtime_id to get a fresh one \
                 (its scratch directory starts empty)"
            ),
            Self::Capacity(m) => m.clone(),
            Self::Timeout => "execution exceeded its deadline".into(),
            Self::HandlerError(m) => m.clone(),
            Self::ResourceExhausted(m) => m.clone(),
            Self::Engine(m) => m.clone(),
        }
    }
}

impl std::fmt::Display for CodeRunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for CodeRunnerError {}

impl From<CodeRunnerError> for Error {
    fn from(e: CodeRunnerError) -> Self {
        Error::Handler(e.to_string())
    }
}

/// Rewrite an error that would disclose a `runtime_id` the receiving caller
/// never held.
///
/// `RuntimeNotFound`/`Expired` name their id deliberately — to the holder.
/// A caller who ran one-shot, or whose call was routed through a namespace
/// runtime they never addressed, is not that holder.
pub fn redact_unheld(e: CodeRunnerError) -> CodeRunnerError {
    match e {
        CodeRunnerError::RuntimeNotFound(_) | CodeRunnerError::Expired(_) => {
            CodeRunnerError::Engine("the run's runtime was reaped or lost mid-run; retry".into())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_wire_strings() {
        for (e, code) in [
            (
                CodeRunnerError::InvalidRequest("x".into()),
                "code-runner::invalid_request",
            ),
            (
                CodeRunnerError::RuntimeNotFound("r".into()),
                "code-runner::runtime_not_found",
            ),
            (
                CodeRunnerError::NamespaceNotFound("n".into()),
                "code-runner::runtime_not_found",
            ),
            (CodeRunnerError::Expired("r".into()), "code-runner::expired"),
            (
                CodeRunnerError::Capacity("x".into()),
                "code-runner::capacity",
            ),
            (CodeRunnerError::Timeout, "code-runner::timeout"),
            (
                CodeRunnerError::HandlerError("x".into()),
                "code-runner::handler_error",
            ),
            (
                CodeRunnerError::ResourceExhausted("x".into()),
                "code-runner::resource_exhausted",
            ),
            (CodeRunnerError::Engine("x".into()), "code-runner::engine"),
        ] {
            assert_eq!(e.code(), code, "wrong code for {e:?}");
        }
    }

    #[test]
    fn display_is_code_colon_message() {
        assert_eq!(
            CodeRunnerError::RuntimeNotFound("rt-7".into()).to_string(),
            "code-runner::runtime_not_found: unknown runtime_id rt-7"
        );
    }

    /// A caller who never supplied a runtime_id must not learn one. Mutation:
    /// make `redact_unheld` the identity function.
    #[test]
    fn an_unheld_runtime_id_is_never_disclosed() {
        let redacted = redact_unheld(CodeRunnerError::Expired("rt-secret".into()));
        assert!(!redacted.to_string().contains("rt-secret"));
        assert_eq!(redacted.code(), "code-runner::engine");
        // Everything else passes through untouched.
        assert_eq!(
            redact_unheld(CodeRunnerError::Timeout).code(),
            "code-runner::timeout"
        );
    }
}
