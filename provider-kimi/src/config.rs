//! Effective per-request config: credential + url + max_tokens.
//! Precedence for max_tokens: router-resolved effective budget
//! (`ProviderStreamInput.max_output_tokens`) → the operator's configured
//! `max_tokens` (from resolve) → the worker default.
use llm_router::types::credential::Credential;
use llm_router::types::router::ProviderResolveResponse;

pub const DEFAULT_API_URL: &str = "https://api.moonshot.ai/v1/chat/completions";
pub const DEFAULT_MAX_TOKENS: u64 = 8192;

#[derive(Debug, Clone)]
pub struct KimiConfig {
    pub credential_value: String,
    pub model: String,
    pub max_tokens: u64,
    pub api_url: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    NotConfigured,
    InvalidApiUrl(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotConfigured => f.write_str(
                "provider kimi not configured (no api_key in the llm-router entry and MOONSHOT_API_KEY unset)",
            ),
            ConfigError::InvalidApiUrl(url) => write!(
                f,
                "provider kimi has an invalid endpoint url: {url:?} (must be an absolute http(s) URL)"
            ),
        }
    }
}

/// The single Credential → bearer secret mapping; streaming and discovery
/// must agree on it. Moonshot takes `Authorization: Bearer` for both shapes.
pub fn credential_parts(credential: &Credential) -> &str {
    match credential {
        Credential::ApiKey { key } => key,
        Credential::Oauth { access_token, .. } => access_token,
    }
}

pub fn config_from_resolve(
    model: &str,
    effective_max_tokens: Option<u64>,
    resolved: &ProviderResolveResponse,
) -> Result<KimiConfig, ConfigError> {
    let credential_value = match &resolved.credential {
        Some(credential) => credential_parts(credential).trim().to_string(),
        None => return Err(ConfigError::NotConfigured),
    };
    if credential_value.is_empty() {
        return Err(ConfigError::NotConfigured);
    }
    let api_url = match resolved.api_url.as_deref().map(str::trim) {
        Some(url) if !url.is_empty() => url.to_string(),
        _ => DEFAULT_API_URL.to_string(),
    };
    match reqwest::Url::parse(&api_url) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => {}
        _ => return Err(ConfigError::InvalidApiUrl(api_url)),
    }
    Ok(KimiConfig {
        credential_value,
        model: model.to_string(),
        max_tokens: effective_max_tokens
            .or(resolved.max_tokens)
            .unwrap_or(DEFAULT_MAX_TOKENS),
        api_url,
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
            ConfigError::NotConfigured
        );
    }

    #[test]
    fn trims_credentials_and_rejects_blank_values() {
        let cfg = config_from_resolve(
            "m",
            None,
            &resolved(
                Some(Credential::ApiKey {
                    key: " sk\n".into(),
                }),
                None,
            ),
        )
        .unwrap();
        assert_eq!(cfg.credential_value, "sk");

        let err = config_from_resolve(
            "m",
            None,
            &resolved(Some(Credential::ApiKey { key: " \n".into() }), None),
        )
        .unwrap_err();
        assert_eq!(err, ConfigError::NotConfigured);
    }

    #[test]
    fn trims_and_validates_api_url() {
        let mut response = resolved(Some(Credential::ApiKey { key: "sk".into() }), None);
        response.api_url = Some(" https://proxy.example/v1/chat/completions ".into());
        assert_eq!(
            config_from_resolve("m", None, &response).unwrap().api_url,
            "https://proxy.example/v1/chat/completions"
        );

        response.api_url = Some("localhost:1234".into());
        assert_eq!(
            config_from_resolve("m", None, &response).unwrap_err(),
            ConfigError::InvalidApiUrl("localhost:1234".into())
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
    fn oauth_credential_yields_its_access_token() {
        let cred = Some(Credential::Oauth {
            access_token: "at".into(),
            refresh_token: None,
            expires_at: None,
            scopes: None,
            provider_extra: None,
        });
        let cfg = config_from_resolve("m", None, &resolved(cred, None)).unwrap();
        assert_eq!(cfg.credential_value, "at");
    }
}
