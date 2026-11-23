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

    /// The harness-owned projects store could not be read or written.
    #[error("harness/project_store: {0}")]
    ProjectStore(String),

    /// The harness-owned saved-folder catalogue could not be read or written.
    #[error("harness/folder_store: {0}")]
    FolderStore(String),

    /// A request the projects API refuses with a code a client acts on
    /// (`name_taken`, `folder_taken`, `not_found`, ...). The message is ONE
    /// JSON object, `{"code", "message", ...}`, as tmux gives its errors, so
    /// a client parses it from the handler error. Build it with
    /// [`HarnessError::rejected`].
    #[error("{0}")]
    Rejected(String),

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
            HarnessError::ProjectStore(_) => "harness/project_store",
            HarnessError::FolderStore(_) => "harness/folder_store",
            HarnessError::Rejected(_) => "harness/rejected",
            HarnessError::Internal(_) => "harness/internal",
        }
    }
}

impl HarnessError {
    /// A [`HarnessError::Rejected`]: `{"code": code, "message": message}`
    /// plus every field of `extra` (an object).
    pub fn rejected(code: &str, message: impl Into<String>, extra: serde_json::Value) -> Self {
        let mut body = serde_json::json!({ "code": code, "message": message.into() });
        if let (Some(body), serde_json::Value::Object(extra)) = (body.as_object_mut(), extra) {
            body.extend(extra);
        }
        HarnessError::Rejected(body.to_string())
    }

    /// The client-facing code of a [`HarnessError::Rejected`] (`name_taken`, ...).
    pub fn rejected_code(&self) -> Option<String> {
        match self {
            HarnessError::Rejected(wire) => serde_json::from_str::<serde_json::Value>(wire)
                .ok()?
                .get("code")?
                .as_str()
                .map(str::to_string),
            _ => None,
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
            HarnessError::ProjectStore("m".into()),
            HarnessError::FolderStore("m".into()),
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

    /// A rejection is one JSON object, as tmux's errors are, so clients read
    /// its `code` from the handler error.
    #[test]
    fn a_rejection_is_one_json_object_with_its_code() {
        let error = HarnessError::rejected(
            "folder_taken",
            "/w/api is already in “billing”",
            serde_json::json!({ "project_id": "prj_1", "project_name": "billing" }),
        );
        let wire: serde_json::Value = serde_json::from_str(&error.to_string()).unwrap();
        assert_eq!(wire["code"], "folder_taken");
        assert_eq!(wire["project_name"], "billing");
        assert_eq!(error.rejected_code().as_deref(), Some("folder_taken"));
        assert_eq!(error.code(), "harness/rejected");
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
