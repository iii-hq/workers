//! Upstream failure → shared ErrorKind taxonomy (spec § provider protocol
//! rule 5: five providers MUST NOT invent five taxonomies).
use iii_sdk::errors::Error;
use llm_router::types::events::ErrorKind;
use serde_json::Value;

/// Map a Groq HTTP status + error body to the shared taxonomy
/// (api-docs.groq.com, quick_start/error_codes).
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
        // 413 is an oversized request body, which for a chat request means
        // the prompt did not fit.
        Some(413) => ErrorKind::ContextOverflow,
        // 498 is Groq's own code for flex-tier capacity being exhausted: the
        // request was not served and the same request may be served later, so
        // it belongs with the retryable statuses rather than the caller bugs.
        Some(498) => ErrorKind::Transient,
        // 499 is a cancellation by the caller. Nothing failed upstream and a
        // retry is the caller's decision, not the router's.
        Some(499) => ErrorKind::Permanent,
        // 500, 502 and 503 all say retry after a wait; Groq does not bill for
        // them.
        Some(s) if s >= 500 => ErrorKind::Transient,
        // 400 (bad body) and 422 (invalid parameters) are caller bugs unless
        // the message says the prompt simply did not fit.
        Some(_) if is_context_overflow_message(message) => ErrorKind::ContextOverflow,
        Some(_) => ErrorKind::Permanent,
        None => ErrorKind::Transient,
    }
}

/// Whether a rate-limit message describes a single request too large for the
/// whole per-minute allowance, rather than a spent quota.
///
/// Groq phrases it as `... (TPM): Limit 8000, Requested 39082, please reduce
/// your message size ...`. `None` when the message carries no such pair, which
/// leaves the caller on the ordinary rate-limit reading.
fn requested_exceeds_limit(msg: &str) -> Option<bool> {
    let number_after = |label: &str| -> Option<u64> {
        let rest = msg.split_once(label)?.1.trim_start();
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    let limit = number_after("Limit")?;
    let requested = number_after("Requested")?;
    Some(requested > limit)
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

/// Groq sends OpenAI-style envelopes `{ "error": { "message", "type",
/// "code" } }`; a custom OpenAI-compatible endpoint behind an `api_url`
/// override may use the same vocabulary, so both are honored. A bare string
/// `error` field is sniffed for overflow phrasing as a last resort.
fn classify_error_value(v: &Value, status: Option<u16>) -> Option<ErrorKind> {
    let err = v.get("error")?;
    // Envelope 2: `error` is a bare string message — sniff it directly.
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
        // A per-minute token budget, reported as HTTP 413. The status alone
        // reads as "too big for the model", so the code has to be read — but
        // the code alone is not enough either, because two different failures
        // share it.
        //
        // If the quota for the minute is merely spent, waiting fixes it and
        // this is a rate limit. If one request is larger than the entire
        // per-minute allowance, waiting fixes nothing: no amount of backoff
        // makes a 39k request fit an 8k budget, and retrying only burns the
        // turn. Groq states both numbers, so the request is compared against
        // the limit and the answer follows from that — a prompt that must
        // shrink is an overflow, whatever the code says.
        "rate_limit_exceeded" => {
            return Some(match requested_exceeds_limit(msg) {
                Some(true) => ErrorKind::ContextOverflow,
                _ => ErrorKind::RateLimited,
            })
        }
        // Billing walls, not rate limits: the router's backoff cannot fix them.
        "insufficient_quota" | "insufficient_balance" => return Some(ErrorKind::Permanent),
        "invalid_api_key" | "authentication_error" | "account_deactivated" => {
            return Some(ErrorKind::AuthExpired)
        }
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

/// Invalid handler input surfaced on the bus in the `{ code, message }`
/// convention (same shape RouterError uses on the router side).
pub fn invalid_request(message: impl Into<String>) -> Error {
    Error::Remote {
        code: "provider/invalid_request".to_string(),
        message: message.into(),
        stacktrace: None,
    }
}

/// A refresh that could not reach the upstream listing. Distinct from an
/// invalid request: the router keeps the previous catalog slice instead of
/// pruning it to empty on a network blip.
pub fn upstream_unavailable(message: impl Into<String>) -> Error {
    Error::Remote {
        code: "provider/upstream_unavailable".to_string(),
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
        assert_eq!(classify(Some(422), "bad params"), ErrorKind::Permanent);
        assert_eq!(classify(None, "connect refused"), ErrorKind::Transient);
    }

    #[test]
    fn groqs_own_status_codes_route_by_whether_a_retry_could_help() {
        // 498 is flex-tier capacity exhausted: nothing was served and the same
        // request may well be served later.
        assert_eq!(classify(Some(498), ""), ErrorKind::Transient);
        assert!(classify(Some(498), "").is_retryable());
        // 499 is the caller cancelling. Retrying it would resurrect work
        // somebody deliberately stopped.
        assert_eq!(classify(Some(499), ""), ErrorKind::Permanent);
        assert!(!classify(Some(499), "").is_retryable());
    }

    #[test]
    fn a_quota_envelope_is_permanent_whatever_the_status() {
        let body = r#"{"error":{"message":"quota exceeded","type":"invalid_request_error","code":"insufficient_quota"}}"#;
        assert_eq!(classify(Some(400), body), ErrorKind::Permanent);
        assert!(!classify(Some(400), body).is_retryable());
    }

    #[test]
    fn a_request_bigger_than_the_whole_budget_is_an_overflow_not_a_rate_limit() {
        // Captured live, and retried three times before this distinction
        // existed: no amount of backoff makes a 39082-token request fit an
        // 8000-token minute. Only a smaller prompt does, which is what the
        // upstream is asking for.
        let body = r#"{"error":{"message":"Request too large for model `openai/gpt-oss-120b` in organization `org_x` service tier `on_demand` on tokens per minute (TPM): Limit 8000, Requested 39082, please reduce your message size and try again.","type":"tokens","code":"rate_limit_exceeded"}}"#;
        assert_eq!(classify(Some(413), body), ErrorKind::ContextOverflow);
        assert!(!classify(Some(413), body).is_retryable());
    }

    #[test]
    fn a_merely_spent_quota_stays_a_retryable_rate_limit() {
        // The other half of the same code: the request fits the budget, the
        // budget is just used up for now, and waiting is exactly the fix.
        let body = r#"{"error":{"message":"Rate limit reached for model `llama-3.3-70b-versatile` on tokens per minute (TPM): Limit 300000, Requested 12000, please try again in 1.5s.","type":"tokens","code":"rate_limit_exceeded"}}"#;
        assert_eq!(classify(Some(429), body), ErrorKind::RateLimited);
        assert!(classify(Some(429), body).is_retryable());

        // No numbers to compare: the ordinary rate-limit reading stands.
        let bare = r#"{"error":{"message":"Rate limit reached","type":"tokens","code":"rate_limit_exceeded"}}"#;
        assert_eq!(classify(Some(429), bare), ErrorKind::RateLimited);

        // A 413 with nothing to read still means the prompt did not fit.
        assert_eq!(classify(Some(413), ""), ErrorKind::ContextOverflow);
    }

    #[test]
    fn openai_style_envelope_codes_are_honored() {
        let body = r#"{"error":{"message":"This model's maximum context length is 65536 tokens.","type":"invalid_request_error","code":"context_length_exceeded"}}"#;
        assert_eq!(classify(Some(400), body), ErrorKind::ContextOverflow);

        let body = r#"{"error":{"message":"Authentication Fails, Your api key: sk-x is invalid","type":"authentication_error","code":"invalid_request_error"}}"#;
        assert_eq!(classify(Some(401), body), ErrorKind::AuthExpired);

        let body = r#"{"error":{"message":"The server is overloaded","type":"server_error"}}"#;
        assert_eq!(classify(Some(503), body), ErrorKind::Transient);
    }

    #[test]
    fn bare_string_error_envelope_is_sniffed_for_overflow() {
        let body = r#"{"code":"invalid-argument","error":"This model's maximum prompt length is 65536 but the request contains 90000 tokens."}"#;
        assert_eq!(classify(Some(400), body), ErrorKind::ContextOverflow);
    }

    #[test]
    fn context_overflow_detected_from_message_on_4xx() {
        assert_eq!(
            classify(
                Some(400),
                "This model's maximum context length is 65536 tokens"
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
