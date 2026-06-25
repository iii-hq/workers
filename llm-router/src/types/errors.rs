use serde::{Deserialize, Serialize};

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
        }
    }
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
    }
}
