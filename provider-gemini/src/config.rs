//! Effective per-request config: credential + url + max_tokens.
//! Precedence for max_tokens: router-resolved effective budget
//! (`ProviderStreamInput.max_output_tokens`) → the operator's configured
//! `max_tokens` (from resolve) → the worker default.
use llm_router::types::credential::Credential;
use llm_router::types::router::ProviderResolveResponse;
use std::time::Duration;

/// Operator-controlled upstream wait budget; shared by transport reads and
/// generation startup so slow gateways can raise both limits together.
pub(crate) fn upstream_timeout() -> Duration {
    std::env::var("PROVIDER_READ_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map(Duration::from_secs)
        // Google's Carbon and Neon migration notes ask for client timeouts of at
        // least 15 minutes on hard prompts. Upstream is silent while it
        // generates a function call's arguments (99 s live for 9.8k output
        // tokens), and the first-token deadline shares this value.
        .unwrap_or(Duration::from_secs(900))
}

// `v1` answers 404 for gemini-3.1-pro-preview, gemini-3-flash-preview and the
// gemini-flash-latest, gemini-flash-lite-latest and gemini-pro-latest aliases;
// `v1beta` serves every listed id. `api_url` is the full endpoint URL.
pub const DEFAULT_API_URL: &str = "https://generativelanguage.googleapis.com/v1beta/interactions";
// `max_output_tokens` covers thinking plus output; 65,536 is the documented
// default cap.
pub const DEFAULT_MAX_TOKENS: u64 = 65_536;
/// Above this a foreground stream ends `requires_continuation` and expects a
/// continuation loop this provider does not implement.
pub const SYNC_OUTPUT_CAP: u64 = 65_536;

#[derive(Debug, Clone)]
pub struct GeminiConfig {
    /// The auth header: name and value.
    pub auth: (&'static str, String),
    pub model: String,
    pub max_tokens: u64,
    pub api_url: String,
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
    /// The resolved credential is OAuth, which needs a project header the
    /// router's credential does not carry.
    OauthUnsupported,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotConfigured => f.write_str(
                "provider gemini not configured (no api_key in the llm-router entry \
                 and GEMINI_API_KEY unset)",
            ),
            ConfigError::InvalidApiUrl(u) => write!(
                f,
                "provider gemini has an invalid endpoint url: {u:?} \
                 (must be an absolute http(s) URL)"
            ),
            ConfigError::OauthUnsupported => {
                f.write_str("OAuth credentials are not supported by provider-gemini yet")
            }
        }
    }
}

/// The single Credential → auth header mapping; streaming and discovery must
/// agree on it. OAuth would need `Authorization: Bearer` plus
/// `x-goog-user-project`, and `Credential::Oauth` has no project id.
pub fn auth_header(credential: &Credential) -> Result<(&'static str, String), ConfigError> {
    match credential {
        Credential::ApiKey { key } => Ok(("x-goog-api-key", key.trim().to_string())),
        Credential::Oauth { .. } => Err(ConfigError::OauthUnsupported),
    }
}

pub fn config_from_resolve(
    model: &str,
    effective_max_tokens: Option<u64>,
    resolved: &ProviderResolveResponse,
) -> Result<GeminiConfig, ConfigError> {
    // Trim both config-sourced values: a credential pasted with a trailing
    // newline makes an invalid `Authorization` header value, and a stray space
    // breaks URL parsing — both surface only as an opaque reqwest "builder
    // error" at send time.
    let auth = match &resolved.credential {
        Some(credential) => auth_header(credential)?,
        None => return Err(ConfigError::NotConfigured),
    };
    if auth.1.is_empty() {
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
    Ok(GeminiConfig {
        auth,
        model: model.to_string(),
        max_tokens: effective_max_tokens
            .or(resolved.max_tokens)
            .unwrap_or(DEFAULT_MAX_TOKENS)
            .min(SYNC_OUTPUT_CAP),
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
        assert_eq!(cfg.auth, ("x-goog-api-key", "sk-abc".to_string()));
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
    fn oauth_credential_is_refused() {
        let cred = Credential::Oauth {
            access_token: "at".into(),
            refresh_token: None,
            expires_at: None,
            scopes: None,
            provider_extra: None,
        };
        assert_eq!(auth_header(&cred), Err(ConfigError::OauthUnsupported));
        assert_eq!(
            config_from_resolve("m", None, &resolved(Some(cred), None)).unwrap_err(),
            ConfigError::OauthUnsupported
        );
    }

    #[test]
    fn max_tokens_is_clamped_to_the_sync_cap() {
        let cfg = config_from_resolve("m", Some(200_000), &resolved(some_key(), None)).unwrap();
        assert_eq!(cfg.max_tokens, SYNC_OUTPUT_CAP);
        let cfg = config_from_resolve("m", None, &resolved(some_key(), Some(100_000))).unwrap();
        assert_eq!(cfg.max_tokens, SYNC_OUTPUT_CAP);
    }

    #[test]
    fn default_endpoint_is_v1beta_and_timeout_is_fifteen_minutes() {
        assert!(DEFAULT_API_URL.ends_with("/v1beta/interactions"));
        if std::env::var("PROVIDER_READ_TIMEOUT_SECS").is_err() {
            assert_eq!(upstream_timeout(), Duration::from_secs(900));
        }
    }
}
