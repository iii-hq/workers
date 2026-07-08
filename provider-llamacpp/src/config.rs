//! Effective per-request config: credential + url + max_tokens.
//! Precedence for max_tokens: router-resolved effective budget
//! (`ProviderStreamInput.max_output_tokens`) → the operator's configured
//! `max_tokens` (from resolve) → the worker default.
use llm_router::types::credential::Credential;
use llm_router::types::router::ProviderResolveResponse;

// llama-server's own default bind address/port. Unlike every cloud
// provider, a self-hosted llama.cpp server commonly runs with no
// `--api-key` at all — see `credential_value` below, which is optional.
pub const DEFAULT_API_URL: &str = "http://127.0.0.1:8080/v1/chat/completions";
pub const DEFAULT_MAX_TOKENS: u64 = 8192;

#[derive(Debug, Clone)]
pub struct LlamacppConfig {
    /// `None` when the server was resolved with no credential — the normal
    /// case for a local llama.cpp server started without `--api-key`.
    /// Streaming and discovery both send no `Authorization` header in that
    /// case rather than treating it as a configuration error.
    pub credential_value: Option<String>,
    pub model: String,
    pub max_tokens: u64,
    pub api_url: String,
}

/// Why an effective config could not be built — the caller turns each into a
/// permanent error frame with a message that names the actual problem.
#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// `api_url` is set but is not an absolute http(s) URL. Carries the
    /// offending value so the error frame can show it (a reqwest "builder
    /// error" otherwise hides which value was bad).
    InvalidApiUrl(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::InvalidApiUrl(u) => write!(
                f,
                "provider llamacpp has an invalid endpoint url: {u:?} \
                 (must be an absolute http(s) URL)"
            ),
        }
    }
}

/// The single Credential → bearer secret mapping; streaming and discovery
/// must agree on it. llama.cpp's `--api-key` takes `Authorization: Bearer`.
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
) -> Result<LlamacppConfig, ConfigError> {
    // Trim: a credential pasted with a trailing newline makes an invalid
    // `Authorization` header value, and a stray space breaks URL parsing —
    // both surface only as an opaque reqwest "builder error" at send time.
    // Blank-after-trim is treated the same as "no credential configured",
    // not an error: most local llama.cpp servers run with no `--api-key`.
    let credential_value = resolved
        .credential
        .as_ref()
        .map(|c| credential_parts(c).trim().to_string())
        .filter(|s| !s.is_empty());
    let api_url = match resolved.api_url.as_deref().map(str::trim) {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => DEFAULT_API_URL.to_string(),
    };
    // Reject anything reqwest can't build a request from, with a clear message.
    match reqwest::Url::parse(&api_url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => {}
        _ => return Err(ConfigError::InvalidApiUrl(api_url)),
    }
    Ok(LlamacppConfig {
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
    fn missing_credential_resolves_with_no_auth_header_needed() {
        // The common case: a local llama.cpp server with no --api-key.
        let cfg = config_from_resolve("m", None, &resolved(None, None)).unwrap();
        assert_eq!(cfg.credential_value, None);
        assert_eq!(cfg.api_url, DEFAULT_API_URL);
    }

    #[test]
    fn credential_is_trimmed() {
        // A key pasted with a trailing newline must not poison the auth header.
        let cred = Some(Credential::ApiKey {
            key: "sk-abc\n".into(),
        });
        let cfg = config_from_resolve("m", None, &resolved(cred, None)).unwrap();
        assert_eq!(cfg.credential_value.as_deref(), Some("sk-abc"));
    }

    #[test]
    fn whitespace_only_credential_is_treated_as_absent() {
        let cred = Some(Credential::ApiKey { key: "  \n".into() });
        let cfg = config_from_resolve("m", None, &resolved(cred, None)).unwrap();
        assert_eq!(cfg.credential_value, None);
    }

    #[test]
    fn api_url_is_trimmed_and_kept() {
        let cfg = config_from_resolve(
            "m",
            None,
            &resolved_with_url(some_key(), Some(" http://h/v1 ")),
        )
        .unwrap();
        assert_eq!(cfg.api_url, "http://h/v1");
    }

    #[test]
    fn blank_api_url_override_falls_back_to_default() {
        let cfg =
            config_from_resolve("m", None, &resolved_with_url(some_key(), Some("   "))).unwrap();
        assert_eq!(cfg.api_url, DEFAULT_API_URL);
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
        assert_eq!(cfg.credential_value.as_deref(), Some("at"));
    }
}
