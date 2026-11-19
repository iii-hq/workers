//! Upstream failure → shared ErrorKind taxonomy (spec § provider protocol
//! rule 5: five providers MUST NOT invent five taxonomies).
use iii_sdk::errors::Error;
use llm_router::types::events::ErrorKind;
use serde_json::Value;

/// The `error` object of an error body, in any of the three shapes the API
/// sends: plain JSON, SSE-framed (`data: {...}`), or google.rpc wrapped in a
/// one-element array (the interactions route) or bare (the models route).
pub fn error_parts(body: &str) -> Option<Value> {
    let parsed = serde_json::from_str::<Value>(body).ok().or_else(|| {
        let data = body
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .next_back()?;
        serde_json::from_str(data).ok()
    })?;
    let parsed = match parsed {
        Value::Array(mut items) if !items.is_empty() => items.swap_remove(0),
        other => other,
    };
    parsed.get("error").cloned()
}

/// The message for an error frame: `error.message` when the body parsed,
/// else the raw body, else `gemini http <status>`.
pub fn error_message(status: Option<u16>, body: &str) -> String {
    match error_parts(body)
        .as_ref()
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
    {
        Some(m) => m.to_string(),
        None if !body.is_empty() => body.to_string(),
        None => match status {
            Some(s) => format!("gemini http {s}"),
            None => "gemini upstream error".to_string(),
        },
    }
}

/// Map an HTTP status + error body to the shared taxonomy. `None` status =
/// no HTTP status applies: a connect/read failure, or an error inside a 200
/// stream.
pub fn classify(status: Option<u16>, body: &str) -> ErrorKind {
    match error_parts(body) {
        Some(error) => classify_error(status, &error),
        None => classify_status(status, body),
    }
}

/// One `error` object (HTTP body, SSE `error` event, `interaction.errors[]`).
pub fn classify_error(status: Option<u16>, error: &Value) -> ErrorKind {
    let message = error.get("message").and_then(Value::as_str).unwrap_or("");
    if let Some(code) = error.get("code").and_then(Value::as_str) {
        if let Some(kind) = classify_code(code, message) {
            return kind;
        }
    } else if error
        .get("details")
        .and_then(Value::as_array)
        .is_some_and(|d| {
            d.iter()
                .any(|x| x.get("reason").and_then(Value::as_str) == Some("API_KEY_INVALID"))
        })
    {
        // A bad key is HTTP 400; the status alone would say Permanent and the
        // credential cache would never be dropped.
        return ErrorKind::AuthExpired;
    }
    // A google.rpc error carries the HTTP status as an integer `code`; inside
    // a 200 stream it is the only status there is. A small gRPC code is not one.
    let status = status.or_else(|| {
        error
            .get("code")
            .and_then(Value::as_u64)
            .filter(|c| (100..600).contains(c))
            .map(|c| c as u16)
    });
    // google.rpc `status` is not read: RESOURCE_EXHAUSTED on a 402 means the
    // prepaid credits are gone, not a rate limit.
    match status {
        Some(_) => classify_status(status, message),
        None => ErrorKind::Transient,
    }
}

/// The Interactions `error.code` table; `None` for an unknown code.
fn classify_code(code: &str, message: &str) -> Option<ErrorKind> {
    Some(match code {
        "invalid_request" | "bad_request" if is_context_overflow_message(message) => {
            ErrorKind::ContextOverflow
        }
        "invalid_request" | "bad_request" => ErrorKind::Permanent,
        "authentication" | "permission_denied" | "unauthorized" | "forbidden" => {
            ErrorKind::AuthExpired
        }
        "rate_limit_exceeded" | "too_many_requests" => ErrorKind::RateLimited,
        // A daily quota does not clear within the router's backoff.
        "quota_exceeded" => ErrorKind::Permanent,
        "not_found"
        | "model_not_found"
        | "payment_required"
        | "failed_precondition"
        | "parameter_unknown"
        | "out_of_range"
        | "already_exists"
        | "unimplemented"
        | "safety"
        | "recitation"
        | "language"
        | "prohibited_content"
        | "spii"
        | "blocklist"
        | "jailbreak"
        | "content_blocked"
        | "missing_thought_signature"
        | "no_image" => ErrorKind::Permanent,
        c if c.starts_with("image_") => ErrorKind::Permanent,
        // A sampling fault; a retry can succeed.
        "malformed_function_call"
        | "malformed_tool_call"
        | "unexpected_tool_call"
        | "too_many_tool_calls" => ErrorKind::Transient,
        "aborted"
        | "cancelled"
        | "api_error"
        | "service_unavailable"
        | "deadline_exceeded"
        | "gateway_timeout"
        | "bad_gateway"
        | "request_timeout"
        | "internal_server_error"
        | "internal" => ErrorKind::Transient,
        "payload_too_large" | "content_too_large" | "request_entity_too_large" => {
            ErrorKind::ContextOverflow
        }
        _ => return None,
    })
}

fn classify_status(status: Option<u16>, message: &str) -> ErrorKind {
    match status {
        Some(401) | Some(403) => ErrorKind::AuthExpired,
        // A billing wall: retry/backoff cannot fix it.
        Some(402) => ErrorKind::Permanent,
        Some(429) => ErrorKind::RateLimited,
        Some(413) => ErrorKind::ContextOverflow,
        Some(408) => ErrorKind::Transient,
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

fn is_context_overflow_message(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("context length")
        || m.contains("maximum context")
        || m.contains("too many tokens")
        || m.contains("exceeds context")
        || m.contains("context window")
        || m.contains("maximum prompt length")
        || m.contains("prompt is too long")
        // "The input token count exceeds the maximum number of tokens allowed (1048576)."
        || m.contains("token count exceeds")
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

/// Map a serde deserialization failure to the provider's stable
/// `invalid_request` wire error. The shared typed registration adapter keeps
/// schemas emitted while preserving `provider/invalid_request`.
pub fn invalid_request_from_serde(e: serde_json::Error) -> Error {
    invalid_request(format!("bad ProviderStreamInput: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        assert_eq!(classify(Some(402), ""), ErrorKind::Permanent);
        assert_eq!(classify(Some(408), ""), ErrorKind::Transient);
        assert_eq!(classify(None, "connect refused"), ErrorKind::Transient);
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

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/errors/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    #[test]
    fn live_error_bodies_classify() {
        for (name, status, kind) in [
            ("400_sse_framed.txt", 400, ErrorKind::Permanent),
            ("400_bad_key.json", 400, ErrorKind::AuthExpired),
            ("402_credits_depleted.json", 402, ErrorKind::Permanent),
            ("404_unknown_model.json", 404, ErrorKind::Permanent),
            ("404_retired_model.json", 404, ErrorKind::Permanent),
            ("400_overflow.json", 400, ErrorKind::ContextOverflow),
            ("400_unknown_field.json", 400, ErrorKind::Permanent),
            ("400_thinking_level.json", 400, ErrorKind::Permanent),
        ] {
            assert_eq!(classify(Some(status), &fixture(name)), kind, "{name}");
        }
    }

    #[test]
    fn message_is_read_from_each_body_shape() {
        assert_eq!(
            error_message(Some(400), &fixture("400_sse_framed.txt")),
            "Request contains an invalid argument."
        );
        assert_eq!(
            error_message(Some(400), &fixture("400_bad_key.json")),
            "API key not valid. Please pass a valid API key."
        );
        assert_eq!(
            error_message(Some(400), &fixture("400_unknown_field.json")),
            "Unknown parameter 'bogus_field'."
        );
        assert_eq!(error_message(Some(502), "<html>"), "<html>");
        assert_eq!(error_message(Some(502), ""), "gemini http 502");
    }

    #[test]
    fn code_table() {
        let kind = |code: &str| classify_error(None, &json!({"code": code, "message": "m"}));
        assert_eq!(kind("authentication"), ErrorKind::AuthExpired);
        assert_eq!(kind("permission_denied"), ErrorKind::AuthExpired);
        assert_eq!(kind("rate_limit_exceeded"), ErrorKind::RateLimited);
        assert_eq!(kind("quota_exceeded"), ErrorKind::Permanent);
        assert_eq!(kind("safety"), ErrorKind::Permanent);
        assert_eq!(kind("image_too_large"), ErrorKind::Permanent);
        assert_eq!(kind("malformed_function_call"), ErrorKind::Transient);
        assert_eq!(kind("service_unavailable"), ErrorKind::Transient);
        assert_eq!(kind("payload_too_large"), ErrorKind::ContextOverflow);
        assert_eq!(kind("not_found"), ErrorKind::Permanent);
        // The code wins over the HTTP status.
        assert_eq!(
            classify_error(Some(500), &json!({"code": "invalid_request"})),
            ErrorKind::Permanent
        );
    }

    #[test]
    fn unknown_code_falls_back_to_status_else_transient() {
        let e = json!({"code": "brand_new_code", "message": "m"});
        assert_eq!(classify_error(None, &e), ErrorKind::Transient);
        assert_eq!(classify_error(Some(429), &e), ErrorKind::RateLimited);
        assert_eq!(classify_error(Some(400), &e), ErrorKind::Permanent);
    }

    #[test]
    fn integer_code_stands_in_for_a_missing_status() {
        let kind = |e: Value| classify_error(None, &e);
        assert_eq!(
            kind(json!({"code": 400, "status": "INVALID_ARGUMENT"})),
            ErrorKind::Permanent
        );
        assert_eq!(kind(json!({"code": 429})), ErrorKind::RateLimited);
        assert_eq!(kind(json!({"code": 503})), ErrorKind::Transient);
        // gRPC INVALID_ARGUMENT, not an HTTP status
        assert_eq!(kind(json!({"code": 3})), ErrorKind::Transient);
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
