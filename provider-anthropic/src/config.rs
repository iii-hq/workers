//! Effective per-request config: credential + auth mode + url + max_tokens.
//! Precedence for max_tokens: router-resolved effective budget
//! (`ProviderStreamInput.max_output_tokens`) → the operator's configured
//! `max_tokens` (from resolve) → the worker default.
use llm_router::types::credential::Credential;
use llm_router::types::router::ProviderResolveResponse;

pub const DEFAULT_API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const DEFAULT_MAX_TOKENS: u64 = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    ApiKey,
    OauthBearer,
}

#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub credential_value: String,
    pub auth_mode: AuthMode,
    pub model: String,
    pub max_tokens: u64,
    pub api_url: String,
}

/// No usable credential — the caller turns this into a permanent error frame.
#[derive(Debug, PartialEq, Eq)]
pub struct NotConfigured;

pub fn config_from_resolve(
    model: &str,
    effective_max_tokens: Option<u64>,
    resolved: &ProviderResolveResponse,
) -> Result<AnthropicConfig, NotConfigured> {
    let (credential_value, auth_mode) = match &resolved.credential {
        Some(Credential::ApiKey { key }) => (key.clone(), AuthMode::ApiKey),
        Some(Credential::Oauth { access_token, .. }) => {
            (access_token.clone(), AuthMode::OauthBearer)
        }
        None => return Err(NotConfigured),
    };
    Ok(AnthropicConfig {
        credential_value,
        auth_mode,
        model: model.to_string(),
        max_tokens: effective_max_tokens
            .or(resolved.max_tokens)
            .unwrap_or(DEFAULT_MAX_TOKENS),
        api_url: resolved
            .api_url
            .clone()
            .unwrap_or_else(|| DEFAULT_API_URL.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::router::CredentialSource;

    fn resolved(
        credential: Option<Credential>,
        max_tokens: Option<u64>,
    ) -> ProviderResolveResponse {
        ProviderResolveResponse {
            configured: credential.is_some(),
            source: CredentialSource::Config,
            credential,
            api_url: None,
            max_tokens,
        }
    }

    #[test]
    fn missing_credential_is_not_configured() {
        assert_eq!(
            config_from_resolve("m", None, &resolved(None, None)).unwrap_err(),
            NotConfigured
        );
    }

    #[test]
    fn max_tokens_precedence_effective_then_configured_then_default() {
        let key = Some(Credential::ApiKey { key: "sk".into() });
        let cfg = config_from_resolve("m", Some(1000), &resolved(key.clone(), Some(2000))).unwrap();
        assert_eq!(cfg.max_tokens, 1000);
        let cfg = config_from_resolve("m", None, &resolved(key.clone(), Some(2000))).unwrap();
        assert_eq!(cfg.max_tokens, 2000);
        let cfg = config_from_resolve("m", None, &resolved(key, None)).unwrap();
        assert_eq!(cfg.max_tokens, DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn oauth_credential_selects_bearer_mode() {
        let cred = Some(Credential::Oauth {
            access_token: "at".into(),
            refresh_token: None,
            expires_at: None,
            scopes: None,
            provider_extra: None,
        });
        let cfg = config_from_resolve("m", None, &resolved(cred, None)).unwrap();
        assert_eq!(cfg.auth_mode, AuthMode::OauthBearer);
        assert_eq!(cfg.credential_value, "at");
    }
}
