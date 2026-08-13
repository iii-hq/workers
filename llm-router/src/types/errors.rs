use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::types::events::ErrorKind;

/// Stable, worker-prefixed snake_case codes (README § Error conventions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouterCode {
    InvalidRequest,
    UnknownProvider,
    NoProviderForModel,
    AmbiguousModel,
    ProviderUnavailable,
    NotConfigured,
    StructuredOutputUnsupported,
    RegistrationRejected, // token mismatch (spec adaptation: identity binding)
    AuthExpired,
    RateLimited,
    ContextOverflow,
    ProviderRejectedRequest,
    UpstreamUnavailable,
    StreamIdleTimeout,
    StreamIncomplete,
    ProviderProtocolError,
    NoTokenCounter,
    NoEmbedProvider,
    BadProviderResponse,
}

impl RouterCode {
    pub fn as_str(self) -> &'static str {
        match self {
            RouterCode::InvalidRequest => "router/invalid_request",
            RouterCode::UnknownProvider => "router/unknown_provider",
            RouterCode::NoProviderForModel => "router/no_provider_for_model",
            RouterCode::AmbiguousModel => "router/ambiguous_model",
            RouterCode::ProviderUnavailable => "router/provider_unavailable",
            RouterCode::NotConfigured => "router/not_configured",
            RouterCode::StructuredOutputUnsupported => "router/structured_output_unsupported",
            RouterCode::RegistrationRejected => "router/registration_rejected",
            RouterCode::AuthExpired => "router/auth_expired",
            RouterCode::RateLimited => "router/rate_limited",
            RouterCode::ContextOverflow => "router/context_overflow",
            RouterCode::ProviderRejectedRequest => "router/provider_rejected_request",
            RouterCode::UpstreamUnavailable => "router/upstream_unavailable",
            RouterCode::StreamIdleTimeout => "router/stream_idle_timeout",
            RouterCode::StreamIncomplete => "router/stream_incomplete",
            RouterCode::ProviderProtocolError => "router/provider_protocol_error",
            RouterCode::NoTokenCounter => "router/no_token_counter",
            RouterCode::NoEmbedProvider => "router/no_embed_provider",
            RouterCode::BadProviderResponse => "router/bad_provider_response",
        }
    }

    pub fn is_known(code: &str) -> bool {
        matches!(
            code,
            "router/invalid_request"
                | "router/unknown_provider"
                | "router/provider_unavailable"
                | "router/model_not_found"
                | "router/ambiguous_model"
                | "router/no_provider_for_model"
                | "router/not_configured"
                | "router/structured_output_unsupported"
                | "router/registration_rejected"
                | "router/auth_expired"
                | "router/rate_limited"
                | "router/context_overflow"
                | "router/provider_rejected_request"
                | "router/upstream_unavailable"
                | "router/stream_idle_timeout"
                | "router/stream_incomplete"
                | "router/provider_protocol_error"
                | "router/no_token_counter"
                | "router/no_embed_provider"
                | "router/bad_provider_response"
        )
    }
}

/// Stable machine-readable failure returned by router surfaces and terminal
/// assistant messages. Legacy fields remain populated for compatibility.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RouterFailure {
    pub code: String,
    pub kind: ErrorKind,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub attempts: u32,
}

impl RouterFailure {
    pub fn new(
        code: RouterCode,
        kind: ErrorKind,
        message: impl Into<String>,
        retryable: bool,
        provider: Option<&str>,
        model: Option<&str>,
        attempts: u32,
    ) -> Self {
        Self {
            code: code.as_str().to_string(),
            kind,
            message: sanitize_failure_message(&message.into()),
            retryable,
            provider: provider.filter(|p| !p.is_empty()).map(str::to_string),
            model: model.filter(|m| !m.is_empty()).map(str::to_string),
            attempts,
        }
    }

    pub fn from_kind(
        kind: ErrorKind,
        message: impl Into<String>,
        retryable: bool,
        provider: Option<&str>,
        model: Option<&str>,
        attempts: u32,
    ) -> Self {
        let code = match kind {
            ErrorKind::AuthExpired => RouterCode::AuthExpired,
            ErrorKind::RateLimited => RouterCode::RateLimited,
            ErrorKind::ContextOverflow => RouterCode::ContextOverflow,
            ErrorKind::Transient => RouterCode::UpstreamUnavailable,
            ErrorKind::Permanent => RouterCode::ProviderRejectedRequest,
        };
        Self::new(code, kind, message, retryable, provider, model, attempts)
    }

    pub fn from_remote(
        code: impl Into<String>,
        kind: ErrorKind,
        message: impl Into<String>,
        retryable: bool,
        provider: Option<&str>,
        model: Option<&str>,
        attempts: u32,
    ) -> Self {
        Self {
            code: code.into(),
            kind,
            message: sanitize_failure_message(&message.into()),
            retryable,
            provider: provider.filter(|p| !p.is_empty()).map(str::to_string),
            model: model.filter(|m| !m.is_empty()).map(str::to_string),
            attempts,
        }
    }
}

/// Public failure text is intentionally bounded and strips common secret
/// shapes. Raw upstream bodies belong in provider-local traces, never in
/// router responses, configuration diagnostics, or persisted state.
pub fn sanitize_failure_message(message: &str) -> String {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return "provider request failed; inspect provider logs".to_string();
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') || trimmed.starts_with('<') {
        return "provider request failed; inspect provider logs".to_string();
    }
    let one_line = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    static SECRETISH: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r#"(?i)(bearer\s+|api[_ -]?key[=: ]+|token[=: ]+|secret[=: ]+)([^\s,;]+)"#,
        )
        .expect("static secret regex")
    });
    let redacted = SECRETISH.replace_all(&one_line, "$1[redacted]");
    const MAX_PUBLIC_MESSAGE_BYTES: usize = 512;
    if redacted.len() <= MAX_PUBLIC_MESSAGE_BYTES {
        return redacted.into_owned();
    }
    let suffix = "…";
    let mut end = MAX_PUBLIC_MESSAGE_BYTES - suffix.len();
    while !redacted.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{suffix}", &redacted[..end])
}

/// Typed pre-stream throw: `{ code, message }` over the bus.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{}: {message}", code.as_str())]
pub struct RouterError {
    pub code: RouterCode,
    pub message: String,
}

impl RouterError {
    pub fn new(code: RouterCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Typed pre-stream throws surface on the bus as `IIIError::Remote`
/// (the engine's `{ code, message }` convention).
impl From<RouterError> for iii_sdk::errors::Error {
    fn from(e: RouterError) -> Self {
        iii_sdk::errors::Error::Remote {
            code: e.code.as_str().to_string(),
            message: e.message,
            stacktrace: None,
        }
    }
}

/// Map a serde deserialization failure (the typed-handler bad-request path) to
/// the router's stable `invalid_request` wire error. Used with
/// `RegisterFunction::new_async_with_bad_request` so typed schemas are emitted
/// while the malformed-payload contract stays `router/invalid_request`.
pub fn invalid_request_from_serde(e: serde_json::Error) -> iii_sdk::errors::Error {
    RouterError::new(RouterCode::InvalidRequest, e.to_string()).into()
}

/// The engine's invocation path reports a missing function as
/// `function_not_found` (engine/src/engine/mod.rs); bare `NOT_FOUND` is the
/// configuration worker's missing-entry code and must not match here.
pub fn is_function_not_found(err: &iii_sdk::errors::Error) -> bool {
    matches!(err, iii_sdk::errors::Error::Remote { code, .. } if code.eq_ignore_ascii_case("function_not_found"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_worker_prefixed_snake_case() {
        assert_eq!(
            RouterCode::AmbiguousModel.as_str(),
            "router/ambiguous_model"
        );
        assert_eq!(
            RouterCode::StructuredOutputUnsupported.as_str(),
            "router/structured_output_unsupported"
        );
        let err = RouterError::new(RouterCode::InvalidRequest, "model is required");
        assert_eq!(err.code.as_str(), "router/invalid_request");
        assert_eq!(err.to_string(), "router/invalid_request: model is required");
        assert_eq!(RouterCode::RateLimited.as_str(), "router/rate_limited");
        assert!(RouterCode::is_known("router/provider_protocol_error"));
        assert!(!RouterCode::is_known("router/provider_made_this_up"));
    }

    #[test]
    fn failure_is_machine_readable_and_omits_unknown_context() {
        let failure = RouterFailure::from_kind(
            ErrorKind::RateLimited,
            "try later",
            true,
            Some("openai"),
            None,
            2,
        );
        let value = serde_json::to_value(failure).unwrap();
        assert_eq!(value["code"], "router/rate_limited");
        assert_eq!(value["kind"], "rate_limited");
        assert_eq!(value["retryable"], true);
        assert_eq!(value["attempts"], 2);
        assert!(value.get("model").is_none());
    }

    #[test]
    fn public_failure_message_is_bounded_and_redacted() {
        assert_eq!(
            sanitize_failure_message(r#"{\"error\":\"raw upstream body\"}"#),
            "provider request failed; inspect provider logs"
        );
        let message = sanitize_failure_message("Bearer top-secret\napi_key=also-secret");
        assert!(!message.contains("top-secret"));
        assert!(!message.contains("also-secret"));
        assert!(sanitize_failure_message(&"x".repeat(600)).len() <= 512);
    }
}
