//! Upstream failure → shared ErrorKind taxonomy (spec § provider protocol
//! rule 5: five providers MUST NOT invent five taxonomies).
use iii_sdk::IIIError;
use llm_router::types::events::ErrorKind;

/// Map an Anthropic HTTP status + error body to the shared taxonomy.
/// `None` status = the request never got a response (connect/read failure).
pub fn classify(status: Option<u16>, message: &str) -> ErrorKind {
    match status {
        Some(401) | Some(403) => ErrorKind::AuthExpired,
        Some(429) => ErrorKind::RateLimited,
        Some(413) => ErrorKind::ContextOverflow,
        Some(s) if s >= 500 => ErrorKind::Transient,
        Some(_) if is_context_overflow(message) => ErrorKind::ContextOverflow,
        Some(_) => ErrorKind::Permanent,
        None => ErrorKind::Transient,
    }
}

fn is_context_overflow(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("context") || m.contains("too large") || m.contains("too many tokens")
}

/// Invalid handler input surfaced on the bus in the `{ code, message }`
/// convention (same shape RouterError uses on the router side).
pub fn invalid_request(message: impl Into<String>) -> IIIError {
    IIIError::Remote {
        code: "provider/invalid_request".to_string(),
        message: message.into(),
        stacktrace: None,
    }
}

/// Discovery hit a transient upstream failure — caller keeps the old slice.
pub fn upstream_unavailable(message: impl Into<String>) -> IIIError {
    IIIError::Remote {
        code: "provider/upstream_unavailable".to_string(),
        message: message.into(),
        stacktrace: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_map_to_the_shared_taxonomy() {
        assert_eq!(classify(Some(401), ""), ErrorKind::AuthExpired);
        assert_eq!(classify(Some(403), ""), ErrorKind::AuthExpired);
        assert_eq!(classify(Some(429), ""), ErrorKind::RateLimited);
        assert_eq!(classify(Some(413), ""), ErrorKind::ContextOverflow);
        assert_eq!(classify(Some(500), ""), ErrorKind::Transient);
        assert_eq!(classify(Some(529), ""), ErrorKind::Transient);
        assert_eq!(classify(Some(400), "bad request"), ErrorKind::Permanent);
        assert_eq!(classify(None, "connect refused"), ErrorKind::Transient);
    }

    #[test]
    fn context_overflow_detected_from_message_on_4xx() {
        assert_eq!(
            classify(Some(400), "prompt is too large: exceeds context window"),
            ErrorKind::ContextOverflow
        );
        assert_eq!(
            classify(Some(400), "too many tokens"),
            ErrorKind::ContextOverflow
        );
        // 5xx wins over message sniffing
        assert_eq!(classify(Some(500), "context blah"), ErrorKind::Transient);
    }

    #[test]
    fn bus_error_codes_are_worker_prefixed() {
        match invalid_request("x") {
            IIIError::Remote { code, .. } => assert_eq!(code, "provider/invalid_request"),
            other => panic!("want Remote, got {other:?}"),
        }
        match upstream_unavailable("x") {
            IIIError::Remote { code, .. } => assert_eq!(code, "provider/upstream_unavailable"),
            other => panic!("want Remote, got {other:?}"),
        }
    }
}
