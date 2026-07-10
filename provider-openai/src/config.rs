//! Effective per-request config: credential + url + max_tokens.
//! Precedence for max_tokens: router-resolved effective budget
//! (`ProviderStreamInput.max_output_tokens`) → the operator's configured
//! `max_tokens` (from resolve) → the worker default.
use llm_router::types::credential::Credential;
use llm_router::types::router::ProviderResolveResponse;

pub const DEFAULT_API_URL: &str = "https://api.openai.com/v1/responses";
pub const DEFAULT_MAX_TOKENS: u64 = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiMode {
    Responses,
    ChatCompletions,
}

impl ApiMode {
    /// The endpoint is the compatibility switch: official OpenAI defaults to
    /// Responses, while an explicitly configured Chat Completions URL keeps
    /// working for gateways that have not implemented Responses.
    pub fn from_url(api_url: &str) -> Self {
        let path = reqwest::Url::parse(api_url)
            .ok()
            .map(|url| url.path().trim_end_matches('/').to_string())
            .unwrap_or_default();
        if path.ends_with("/responses") {
            Self::Responses
        } else {
            Self::ChatCompletions
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpenaiConfig {
    pub credential_value: String,
    pub model: String,
    pub max_tokens: u64,
    pub api_url: String,
    pub api_mode: ApiMode,
}

/// Why an effective config could not be built — the caller turns each into a
/// permanent error frame with a message that names the actual problem.
#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// No usable credential resolved.
    NotConfigured,
    /// `api_url` is set but is not an absolute http(s) URL. Carries the
    /// offending value so the error frame can show it (a reqwest "builder
    /// error" otherwise hides which value was bad).
    InvalidApiUrl(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotConfigured => f.write_str(
                "provider openai not configured (no api_key in the llm-router entry \
                 and OPENAI_API_KEY unset)",
            ),
            ConfigError::InvalidApiUrl(u) => write!(
                f,
                "provider openai has an invalid endpoint url: {u:?} \
                 (must be an absolute http(s) URL)"
            ),
        }
    }
}

/// The single Credential → bearer secret mapping; streaming and discovery
/// must agree on it. OpenAI takes `Authorization: Bearer` for both shapes.
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
) -> Result<OpenaiConfig, ConfigError> {
    // Trim both config-sourced values: a credential pasted with a trailing
    // newline makes an invalid `Authorization` header value, and a stray space
    // breaks URL parsing — both surface only as an opaque reqwest "builder
    // error" at send time.
    let credential_value = match &resolved.credential {
        Some(credential) => credential_parts(credential).trim().to_string(),
        None => return Err(ConfigError::NotConfigured),
    };
    if credential_value.is_empty() {
        return Err(ConfigError::NotConfigured);
    }
    let api_url = match resolved.api_url.as_deref().map(str::trim) {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => DEFAULT_API_URL.to_string(),
    };
    // Reject anything reqwest can't build a request from, with a clear message.
    match reqwest::Url::parse(&api_url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => {}
        _ => return Err(ConfigError::InvalidApiUrl(api_url)),
    }
    let api_mode = ApiMode::from_url(&api_url);
    Ok(OpenaiConfig {
        credential_value,
        model: model.to_string(),
        max_tokens: effective_max_tokens
            .or(resolved.max_tokens)
            .unwrap_or(DEFAULT_MAX_TOKENS),
        api_url,
        api_mode,
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

    /// Build a resolve response with an explicit api_url override.
    fn resolved_with_url(
        credential: Option<Credential>,
        api_url: Option<&str>,
    ) -> ProviderResolveResponse {
        ProviderResolveResponse {
            api_url: api_url.map(str::to_string),
            ..resolved(credential, None)
        }
    }

    fn some_key() -> Option<Credential> {
        Some(Credential::ApiKey { key: "sk".into() })
    }

    #[test]
    fn missing_credential_is_not_configured() {
        assert_eq!(
            config_from_resolve("m", None, &resolved(None, None)).unwrap_err(),
            ConfigError::NotConfigured
        );
    }

    #[test]
    fn credential_is_trimmed() {
        // A key pasted with a trailing newline must not poison the auth header.
        let cred = Some(Credential::ApiKey {
            key: "sk-abc\n".into(),
        });
        let cfg = config_from_resolve("m", None, &resolved(cred, None)).unwrap();
        assert_eq!(cfg.credential_value, "sk-abc");
    }

    #[test]
    fn whitespace_only_credential_is_not_configured() {
        let cred = Some(Credential::ApiKey { key: "  \n".into() });
        assert_eq!(
            config_from_resolve("m", None, &resolved(cred, None)).unwrap_err(),
            ConfigError::NotConfigured
        );
    }

    #[test]
    fn api_url_is_trimmed_and_kept() {
        let cfg = config_from_resolve(
            "m",
            None,
            &resolved_with_url(some_key(), Some(" https://h/v1 ")),
        )
        .unwrap();
        assert_eq!(cfg.api_url, "https://h/v1");
        assert_eq!(cfg.api_mode, ApiMode::ChatCompletions);
    }

    #[test]
    fn blank_api_url_override_falls_back_to_default() {
        let cfg =
            config_from_resolve("m", None, &resolved_with_url(some_key(), Some("   "))).unwrap();
        assert_eq!(cfg.api_url, DEFAULT_API_URL);
        assert_eq!(cfg.api_mode, ApiMode::Responses);
    }

    #[test]
    fn endpoint_path_selects_transport() {
        assert_eq!(
            ApiMode::from_url("https://api.openai.com/v1/responses"),
            ApiMode::Responses
        );
        assert_eq!(
            ApiMode::from_url("https://gateway.test/v1/responses/"),
            ApiMode::Responses
        );
        assert_eq!(
            ApiMode::from_url("https://gateway.test/v1/chat/completions"),
            ApiMode::ChatCompletions
        );
    }

    #[test]
    fn non_http_api_url_is_rejected() {
        // No scheme: reqwest would fail with an opaque "builder error" at send;
        // we reject up front with the offending value.
        let err = config_from_resolve(
            "m",
            None,
            &resolved_with_url(some_key(), Some("localhost:1234")),
        )
        .unwrap_err();
        assert_eq!(err, ConfigError::InvalidApiUrl("localhost:1234".into()));
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
