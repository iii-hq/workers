//! Upstream failure → shared ErrorKind taxonomy (spec § provider protocol
//! rule 5: five providers MUST NOT invent five taxonomies).
use iii_sdk::errors::Error;
use llm_router::types::events::ErrorKind;
use serde_json::Value;

/// Map a Sarvam AI HTTP status + error body to the shared taxonomy.
/// `None` status = the request never got a response (connect/read failure).
pub fn classify(status: Option<u16>, message: &str) -> ErrorKind {
    if let Ok(v) = serde_json::from_str::<Value>(message) {
        if let Some(kind) = classify_error_value(&v, status) {
            return kind;
        }
    }
    match status {
        Some(401) | Some(403) => ErrorKind::AuthExpired,
        Some(429) => ErrorKind::RateLimited,
        Some(413) => ErrorKind::ContextOverflow,
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

/// Sarvam AI sends OpenAI-style envelopes `{ "error": { "message", "code" } }`
/// OpenAI-compatible endpoint behind an `api_url` override may send the
/// OpenAI codes/types instead, so both vocabularies are honored. A bare
/// string `error` field is sniffed for overflow phrasing as a last resort.
fn classify_error_value(v: &Value, status: Option<u16>) -> Option<ErrorKind> {
    let err = v.get("error")?;
    if let Some(text) = err.as_str() {
        if is_context_overflow_message(text) {
            return Some(ErrorKind::ContextOverflow);
        }
        return None;
    }
    let code = err.get("code").and_then(Value::as_str).unwrap_or("");
    let err_type = err.get("type").and_then(Value::as_str).unwrap_or("");
    let msg = err.get("message").and_then(Value::as_str).unwrap_or("");
    match code {
        "context_length_exceeded" => return Some(ErrorKind::ContextOverflow),
        "insufficient_quota" => return Some(ErrorKind::Permanent),
        "invalid_api_key" | "account_deactivated" => return Some(ErrorKind::AuthExpired),
        _ => {}
    }
    match err_type {
        "authentication_error" | "permission_error" => Some(ErrorKind::AuthExpired),
        "rate_limit_error" => Some(ErrorKind::RateLimited),
        "server_error" => Some(ErrorKind::Transient),
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

fn is_context_overflow_message(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("context length")
        || m.contains("maximum context")
        || m.contains("too many tokens")
        || m.contains("exceeds context")
        || m.contains("context window")
        || m.contains("maximum prompt length")
        || m.contains("prompt is too long")
}

/// An HTTP failure from a Sarvam speech endpoint as a coded handler error;
/// the body's `error.message` or `detail` rides along when present.
pub fn upstream_status(status: u16, body: &str) -> Error {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.pointer("/detail"))
                .or_else(|| v.pointer("/message"))
                .and_then(|d| d.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    let code = match status {
        401 | 403 => "provider/auth_expired",
        402 => "provider/quota_exceeded",
        400 | 422 => "provider/invalid_input",
        429 => "provider/rate_limited",
        s if s >= 500 => "provider/upstream_transient",
        _ => "provider/upstream_status",
    };
    Error::Handler(format!("{code}: HTTP {status}: {detail}"))
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

/// Map a serde deserialization failure to the provider's stable
/// `invalid_request` wire error. The shared typed registration adapter keeps
/// schemas emitted while preserving `provider/invalid_request`.
pub fn invalid_request_from_serde(e: serde_json::Error) -> Error {
    invalid_request(format!("bad ProviderStreamInput: {e}"))
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
        assert_eq!(classify(Some(503), ""), ErrorKind::Transient);
        assert_eq!(classify(Some(400), "bad request"), ErrorKind::Permanent);
        assert_eq!(classify(None, "connect refused"), ErrorKind::Transient);
    }

    #[test]
    fn openai_style_envelope_codes_are_honored() {
        let body = r#"{"error":{"message":"This model's maximum context length is 400000 tokens.","type":"invalid_request_error","code":"context_length_exceeded"}}"#;
        assert_eq!(classify(Some(400), body), ErrorKind::ContextOverflow);

        let body = r#"{"error":{"message":"You exceeded your current quota.","type":"insufficient_quota","code":"insufficient_quota"}}"#;
        assert_eq!(classify(Some(429), body), ErrorKind::Permanent);

        let body = r#"{"error":{"message":"Incorrect API key provided.","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        assert_eq!(classify(Some(401), body), ErrorKind::AuthExpired);
    }

    #[test]
    fn prompt_length_overflow_is_context_overflow() {
        let body = r#"{"code":"invalid-argument","error":"This model's maximum prompt length is 256000 but the request contains 295444 tokens."}"#;
        assert_eq!(classify(Some(400), body), ErrorKind::ContextOverflow);
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
        assert_eq!(
            classify(Some(400), r#"tool_call_id "ctx-1" not found in context"#),
            ErrorKind::Permanent
        );
        assert_eq!(classify(Some(500), "context blah"), ErrorKind::Transient);
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
    }
}
