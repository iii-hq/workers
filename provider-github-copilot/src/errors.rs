//! Upstream failure → shared ErrorKind taxonomy (spec § provider protocol
//! rule 5: five providers MUST NOT invent five taxonomies).
//!
//! The Copilot gateway keeps OpenAI-shaped errors but its statuses carry
//! subscription semantics: 401 means the short-lived bearer (or the GitHub
//! login behind it) died; 403 means the plan does not authorize the call
//! (no Copilot access, model not enabled) — permanent, not retryable; 429
//! is quota/rate; 5xx is the gateway or the upstream vendor. Numeric
//! `error.code` envelopes win over the transport status; OpenAI-style string
//! codes from compatibility proxies are honored too.
use iii_sdk::errors::Error;
use llm_router::types::events::ErrorKind;
use serde_json::Value;

/// Map a Copilot HTTP status + error body to the shared taxonomy.
/// `None` status = the request never got a response (connect/read failure).
pub fn classify(status: Option<u16>, message: &str) -> ErrorKind {
    if let Ok(v) = serde_json::from_str::<Value>(message) {
        if let Some(kind) = classify_envelope(&v, status) {
            return kind;
        }
    }
    classify_status(status, message)
}

fn classify_status(status: Option<u16>, message: &str) -> ErrorKind {
    match status {
        Some(401) => ErrorKind::AuthExpired,
        // Billing wall: backoff cannot fix an empty credit balance.
        Some(402) => ErrorKind::Permanent,
        // Moderation flagged the input for this model; retrying the same
        // prompt cannot succeed.
        Some(403) => ErrorKind::Permanent,
        Some(408) => ErrorKind::Transient,
        Some(413) => ErrorKind::ContextOverflow,
        Some(429) => ErrorKind::RateLimited,
        Some(s) if s >= 500 => ErrorKind::Transient,
        Some(_) if is_context_overflow_message(message) => ErrorKind::ContextOverflow,
        Some(_) => ErrorKind::Permanent,
        None => ErrorKind::Transient,
    }
}

/// Map router bus errors surfaced through `router::provider::resolve`.
pub fn classify_bus_error(err: &Error) -> ErrorKind {
    match err {
        Error::Remote { code, .. } if code == "router/registration_rejected" => {
            ErrorKind::Permanent
        }
        _ => ErrorKind::Transient,
    }
}

/// The gateway envelope: `{"error": {"code": <number>, "message"}}` with the
/// HTTP status repeated as the numeric `code`. Vendors' own OpenAI-style
/// string codes can also surface through compatibility proxies; both are
/// honored, with the message text as a final fallback.
fn classify_envelope(v: &Value, status: Option<u16>) -> Option<ErrorKind> {
    let err = v.get("error")?;
    let msg = err.get("message").and_then(Value::as_str).unwrap_or("");

    if let Some(code) = err.get("code").and_then(Value::as_u64) {
        return Some(classify_status(Some(code as u16), msg));
    }

    let code = err.get("code").and_then(Value::as_str).unwrap_or("");
    match code {
        "context_length_exceeded" => return Some(ErrorKind::ContextOverflow),
        // A quota wall is billing, not a rate limit: backoff cannot fix it.
        "insufficient_quota" | "insufficient_credits" => return Some(ErrorKind::Permanent),
        "invalid_api_key" | "invalid_authentication_error" | "account_deactivated" => {
            return Some(ErrorKind::AuthExpired)
        }
        _ => {}
    }

    let err_type = err.get("type").and_then(Value::as_str).unwrap_or("");
    match err_type {
        "invalid_authentication_error"
        | "invalid_api_key"
        | "authentication_error"
        | "permission_error"
        | "permission_denied_error" => Some(ErrorKind::AuthExpired),
        "rate_limit_reached_error" | "rate_limit_error" => Some(ErrorKind::RateLimited),
        "insufficient_quota" => Some(ErrorKind::Permanent),
        "engine_overloaded_error" | "server_error" | "internal_server_error" => {
            Some(ErrorKind::Transient)
        }
        "invalid_request_error" => {
            if status == Some(413) || is_context_overflow_message(msg) {
                Some(ErrorKind::ContextOverflow)
            } else {
                Some(ErrorKind::Permanent)
            }
        }
        _ => None,
    }
}

/// The upstream's "this account cannot call that model" refusal. Entitlement
/// is per-plan and absent from the model listing, so this is the only
/// authoritative signal — the caller prunes the row and tells the operator
/// how to gain access.
pub fn is_model_not_supported(message: &str) -> bool {
    serde_json::from_str::<Value>(message)
        .ok()
        .and_then(|v| {
            v.pointer("/error/code")
                .and_then(Value::as_str)
                .map(|c| c == "model_not_supported")
        })
        .unwrap_or(false)
        || message.contains("model_not_supported")
}

fn is_context_overflow_message(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("context length")
        || m.contains("maximum context")
        || m.contains("too many tokens")
        || m.contains("exceeds context")
        || m.contains("context window")
}

/// Invalid handler input surfaced on the bus in the `{ code, message }`
/// convention (same shape RouterError uses on the router side).
pub fn invalid_request(message: impl Into<String>) -> Error {
    Error::Remote {
        code: "provider/invalid_request".to_string(),
        message: message.into(),
        stacktrace: None,
    }
}

/// Map a serde deserialization failure (the typed-handler bad-request path) to
/// the provider's `invalid_request` wire error. Used with
/// `RegisterFunction::new_async_with_bad_request` so typed schemas are emitted
/// while the malformed-payload contract stays `provider/invalid_request`.
pub fn invalid_request_from_serde(e: serde_json::Error) -> Error {
    invalid_request(format!("bad ProviderStreamInput: {e}"))
}

/// Discovery hit a transient upstream failure — caller keeps the old slice.
pub fn upstream_unavailable(message: impl Into<String>) -> Error {
    Error::Remote {
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
        assert_eq!(classify(Some(402), ""), ErrorKind::Permanent);
        assert_eq!(classify(Some(403), ""), ErrorKind::Permanent);
        assert_eq!(classify(Some(408), ""), ErrorKind::Transient);
        assert_eq!(classify(Some(413), ""), ErrorKind::ContextOverflow);
        assert_eq!(classify(Some(429), ""), ErrorKind::RateLimited);
        assert_eq!(classify(Some(500), ""), ErrorKind::Transient);
        assert_eq!(classify(Some(502), ""), ErrorKind::Transient);
        assert_eq!(classify(Some(503), ""), ErrorKind::Transient);
        assert_eq!(classify(Some(400), "bad request"), ErrorKind::Permanent);
        assert_eq!(classify(None, "connect refused"), ErrorKind::Transient);
    }

    #[test]
    fn numeric_envelope_codes_are_honored() {
        let body = r#"{"error":{"code":402,"message":"Payment required"}}"#;
        assert_eq!(classify(Some(402), body), ErrorKind::Permanent);

        let body = r#"{"error":{"code":403,"message":"Your input was flagged by moderation","metadata":{"reasons":["harassment"]}}}"#;
        assert_eq!(classify(Some(403), body), ErrorKind::Permanent);

        let body = r#"{"error":{"code":429,"message":"Rate limited"}}"#;
        assert_eq!(classify(Some(429), body), ErrorKind::RateLimited);

        let body = r#"{"error":{"code":502,"message":"Provider returned error","metadata":{"provider_name":"SomeVendor"}}}"#;
        assert_eq!(classify(Some(502), body), ErrorKind::Transient);

        // envelope code wins over a disagreeing transport status
        let body = r#"{"error":{"code":401,"message":"No auth credentials found"}}"#;
        assert_eq!(classify(Some(500), body), ErrorKind::AuthExpired);
    }

    #[test]
    fn openai_style_string_codes_from_proxies_are_honored() {
        let body = r#"{"error":{"message":"maximum context length exceeded","code":"context_length_exceeded"}}"#;
        assert_eq!(classify(Some(400), body), ErrorKind::ContextOverflow);

        let body = r#"{"error":{"message":"You exceeded your current quota.","code":"insufficient_quota"}}"#;
        assert_eq!(classify(Some(429), body), ErrorKind::Permanent);

        let body = r#"{"error":{"message":"Invalid Authentication","type":"invalid_authentication_error"}}"#;
        assert_eq!(classify(Some(401), body), ErrorKind::AuthExpired);
    }

    #[test]
    fn context_overflow_detected_from_message_on_4xx() {
        assert_eq!(
            classify(
                Some(400),
                "This model's maximum context length is 128000 tokens"
            ),
            ErrorKind::ContextOverflow
        );
        // generic "context" in tool validation must not false-positive
        assert_eq!(
            classify(Some(400), r#"tool_call_id "ctx-1" not found in context"#),
            ErrorKind::Permanent
        );
        // 5xx wins over message sniffing
        assert_eq!(classify(Some(500), "context blah"), ErrorKind::Transient);
    }

    #[test]
    fn model_not_supported_is_detected_and_permanent() {
        let body = r#"{"error":{"message":"The requested model is not supported.","code":"model_not_supported","param":"model","type":"invalid_request_error"}}"#;
        assert!(is_model_not_supported(body));
        assert_eq!(classify(Some(400), body), ErrorKind::Permanent);
        assert!(!is_model_not_supported(
            r#"{"error":{"code":"rate_limited"}}"#
        ));
        assert!(!is_model_not_supported("plain text"));
    }

    #[test]
    fn registration_rejected_is_permanent_on_the_bus() {
        let err = Error::Remote {
            code: "router/registration_rejected".into(),
            message: "bad token".into(),
            stacktrace: None,
        };
        assert_eq!(classify_bus_error(&err), ErrorKind::Permanent);
        let err = Error::Remote {
            code: "engine/timeout".into(),
            message: "t".into(),
            stacktrace: None,
        };
        assert_eq!(classify_bus_error(&err), ErrorKind::Transient);
    }

    #[test]
    fn bus_error_codes_are_worker_prefixed() {
        match invalid_request("x") {
            Error::Remote { code, .. } => assert_eq!(code, "provider/invalid_request"),
            other => panic!("want Remote, got {other:?}"),
        }
        match upstream_unavailable("x") {
            Error::Remote { code, .. } => assert_eq!(code, "provider/upstream_unavailable"),
            other => panic!("want Remote, got {other:?}"),
        }
    }
}
