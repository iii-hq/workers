//! Error conventions: every error crossing the bus carries a stable
//! snake_case, worker-prefixed code in a `code: message` shape (see
//! tech-specs/2026-06-agentic/README.md § Error conventions).

use iii_sdk::errors::Error;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// The session id is unknown.
    #[error("session/not_found: {0}")]
    NotFound(String),

    /// The entry id is unknown within the session.
    #[error("session/entry_not_found: {0}")]
    EntryNotFound(String),

    /// An explicit `parent_id` does not exist within the session.
    #[error("session/parent_not_found: {0}")]
    ParentNotFound(String),

    /// The operation only applies to `kind: "message"` entries.
    #[error("session/invalid_entry_kind: {0}")]
    InvalidEntryKind(String),

    /// `details` was supplied for a message role that has no `details`.
    #[error("session/details_not_supported: {0}")]
    DetailsNotSupported(String),

    /// `session::append-many` requires at least one message.
    #[error("session/empty_batch: {0}")]
    EmptyBatch(String),

    /// The pagination cursor is malformed or inconsistent with the request.
    #[error("session/invalid_cursor: {0}")]
    InvalidCursor(String),

    /// The request shape is invalid beyond what serde can reject.
    #[error("session/invalid_request: {0}")]
    InvalidRequest(String),

    /// A storage backend call failed.
    #[error("session/storage: {0}")]
    Storage(String),
}

impl SessionError {
    /// The stable error code (the part before `:` on the wire).
    pub fn code(&self) -> &'static str {
        match self {
            SessionError::NotFound(_) => "session/not_found",
            SessionError::EntryNotFound(_) => "session/entry_not_found",
            SessionError::ParentNotFound(_) => "session/parent_not_found",
            SessionError::InvalidEntryKind(_) => "session/invalid_entry_kind",
            SessionError::DetailsNotSupported(_) => "session/details_not_supported",
            SessionError::EmptyBatch(_) => "session/empty_batch",
            SessionError::InvalidCursor(_) => "session/invalid_cursor",
            SessionError::InvalidRequest(_) => "session/invalid_request",
            SessionError::Storage(_) => "session/storage",
        }
    }
}

impl From<SessionError> for Error {
    fn from(e: SessionError) -> Self {
        Error::Handler(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_code_colon_message() {
        let e = SessionError::NotFound("session s_x does not exist".into());
        assert_eq!(
            e.to_string(),
            "session/not_found: session s_x does not exist"
        );
        assert_eq!(e.code(), "session/not_found");
    }

    #[test]
    fn every_variant_display_starts_with_its_code() {
        let variants = vec![
            SessionError::NotFound("m".into()),
            SessionError::EntryNotFound("m".into()),
            SessionError::ParentNotFound("m".into()),
            SessionError::InvalidEntryKind("m".into()),
            SessionError::DetailsNotSupported("m".into()),
            SessionError::EmptyBatch("m".into()),
            SessionError::InvalidCursor("m".into()),
            SessionError::InvalidRequest("m".into()),
            SessionError::Storage("m".into()),
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
