//! Effective per-request config. Unlike the api_key providers there is no
//! operator credential here — the bearer comes from the worker-owned
//! exchange ([`crate::exchange`]) — but the router's resolve step still
//! supplies the operator's `api_url` / `max_tokens` overrides, and the
//! api_url override wins over the endpoint the exchange reply names (the
//! escape hatch for gateways and the hermetic test stub).
//!
//! Precedence for max_tokens: router-resolved effective budget
//! (`ProviderStreamInput.max_output_tokens`) → the operator's configured
//! `max_tokens` (from resolve) → the worker default. For api_url:
//! operator override → exchange reply endpoint → public default.
use crate::exchange::{CopilotBearer, DEFAULT_API_URL};
use llm_router::types::router::ProviderResolveResponse;

pub const DEFAULT_MAX_TOKENS: u64 = 8192;

#[derive(Clone)]
pub struct CopilotConfig {
    pub bearer: String,
    pub model: String,
    pub max_tokens: u64,
    pub api_url: String,
}

/// Manual Debug: the bearer must never reach logs or error text through a
/// diagnostic format of this struct.
impl std::fmt::Debug for CopilotConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopilotConfig")
            .field("bearer", &"<redacted>")
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("api_url", &self.api_url)
            .finish()
    }
}

pub fn build_config(
    model: &str,
    effective_max_tokens: Option<u64>,
    resolved: &ProviderResolveResponse,
    bearer: &CopilotBearer,
) -> CopilotConfig {
    CopilotConfig {
        bearer: bearer.token.clone(),
        model: model.to_string(),
        max_tokens: effective_max_tokens
            .or(resolved.max_tokens)
            .unwrap_or(DEFAULT_MAX_TOKENS),
        api_url: resolved
            .api_url
            .clone()
            .or_else(|| bearer.api_url.clone())
            .unwrap_or_else(|| DEFAULT_API_URL.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::router::CredentialSource;

    fn resolved(api_url: Option<&str>, max_tokens: Option<u64>) -> ProviderResolveResponse {
        ProviderResolveResponse {
            configured: false,
            source: CredentialSource::None,
            credential: None,
            api_url: api_url.map(str::to_string),
            max_tokens,
        }
    }

    fn bearer(api_url: Option<&str>) -> CopilotBearer {
        CopilotBearer {
            token: "tid=x".into(),
            expires_at: 0,
            api_url: api_url.map(str::to_string),
        }
    }

    #[test]
    fn api_url_precedence_operator_then_exchange_then_default() {
        let cfg = build_config(
            "m",
            None,
            &resolved(Some("http://stub/v1/chat/completions"), None),
            &bearer(Some("https://api.exchange.example/chat/completions")),
        );
        assert_eq!(cfg.api_url, "http://stub/v1/chat/completions");
        let cfg = build_config(
            "m",
            None,
            &resolved(None, None),
            &bearer(Some("https://api.exchange.example/chat/completions")),
        );
        assert_eq!(cfg.api_url, "https://api.exchange.example/chat/completions");
        let cfg = build_config("m", None, &resolved(None, None), &bearer(None));
        assert_eq!(cfg.api_url, DEFAULT_API_URL);
    }

    #[test]
    fn max_tokens_precedence_effective_then_configured_then_default() {
        let cfg = build_config("m", Some(1000), &resolved(None, Some(2000)), &bearer(None));
        assert_eq!(cfg.max_tokens, 1000);
        let cfg = build_config("m", None, &resolved(None, Some(2000)), &bearer(None));
        assert_eq!(cfg.max_tokens, 2000);
        let cfg = build_config("m", None, &resolved(None, None), &bearer(None));
        assert_eq!(cfg.max_tokens, DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn debug_output_redacts_the_bearer() {
        let cfg = build_config("m", None, &resolved(None, None), &bearer(None));
        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains("tid=x"));
        assert!(dbg.contains("<redacted>"));
    }
}
