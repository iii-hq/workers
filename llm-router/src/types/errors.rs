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
