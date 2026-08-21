use iii_sdk::errors::Error;
use llm_router::types::events::ErrorKind;
use serde_json::Value;

pub fn classify(status: Option<u16>, body: &str) -> ErrorKind {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        if let Some(kind) = classify_envelope(&value) {
            return kind;
        }
    }
    match status {
        Some(401) => ErrorKind::AuthExpired,
        Some(413) => ErrorKind::ContextOverflow,
        Some(429) => ErrorKind::RateLimited,
        Some(status) if status >= 500 => ErrorKind::Transient,
        Some(_) if is_context_overflow(body) => ErrorKind::ContextOverflow,
        Some(_) => ErrorKind::Permanent,
        None => ErrorKind::Transient,
    }
}

fn classify_envelope(value: &Value) -> Option<ErrorKind> {
    let error = value.get("error")?;
    let has = |candidates: &[&str]| {
        ["type", "code"].iter().any(|field| {
            error
                .get(*field)
                .and_then(Value::as_str)
                .is_some_and(|kind| candidates.contains(&kind))
        })
    };
    if has(&["authentication_error", "invalid_api_key"]) {
        Some(ErrorKind::AuthExpired)
    } else if has(&["rate_limit_error", "rate_limit_exceeded"]) {
        Some(ErrorKind::RateLimited)
    } else if has(&["server_error", "api_error", "overloaded_error"]) {
        Some(ErrorKind::Transient)
    } else if has(&["context_length_exceeded"]) {
        Some(ErrorKind::ContextOverflow)
    } else if has(&[
        "cmd_zdr_no_providers",
        "upgrade_required",
        "invalid_request_error",
    ]) {
        Some(ErrorKind::Permanent)
    } else {
        None
    }
}

fn is_context_overflow(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("context length")
        || message.contains("context window")
        || message.contains("too many tokens")
        || message.contains("maximum context")
}

pub fn classify_bus_error(error: &Error) -> ErrorKind {
    match error {
        Error::Remote { code, .. } if code == "router/registration_rejected" => {
            ErrorKind::Permanent
        }
        _ => ErrorKind::Transient,
    }
}

pub fn invalid_request_from_serde(error: serde_json::Error) -> Error {
    Error::Remote {
        code: "provider/invalid_request".into(),
        message: format!("bad ProviderStreamInput: {error}"),
        stacktrace: None,
    }
}

pub fn upstream_unavailable(message: impl Into<String>) -> Error {
    Error::Remote {
        code: "provider/upstream_unavailable".into(),
        message: message.into(),
        stacktrace: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_errors_map_to_the_shared_taxonomy() {
        assert_eq!(classify(Some(401), ""), ErrorKind::AuthExpired);
        assert_eq!(classify(Some(429), ""), ErrorKind::RateLimited);
        assert_eq!(classify(Some(500), ""), ErrorKind::Transient);
        assert_eq!(
            classify(Some(422), r#"{"error":{"type":"cmd_zdr_no_providers"}}"#),
            ErrorKind::Permanent
        );
        assert_eq!(
            classify(Some(400), "maximum context length exceeded"),
            ErrorKind::ContextOverflow
        );
        assert_eq!(
            classify(
                Some(400),
                r#"{"error":{"type":"invalid_request_error","code":"context_length_exceeded"}}"#
            ),
            ErrorKind::ContextOverflow
        );
    }
}
