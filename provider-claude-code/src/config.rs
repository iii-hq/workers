//! Effective per-request config for the Anthropic Messages API when driven by
//! a Claude Code subscription: OAuth access token + url + max_tokens. The
//! credential comes from the `auth-credentials` vault (`auth::get_token`) or
//! the `~/.claude/.credentials.json` dev fallback; the api_url/max_tokens come
//! from the router's `resolve`. This provider is OAuth-only — API keys belong
//! on `provider-anthropic`.
use llm_router::types::router::ProviderResolveResponse;
use serde_json::Value;

pub const DEFAULT_API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const DEFAULT_MAX_TOKENS: u64 = 8192;

/// Map a namespaced router id to the upstream Anthropic model id
/// (`claude-code/claude-sonnet-4-6` → `claude-sonnet-4-6`).
pub fn upstream_model_id(router_id: &str) -> String {
    router_id
        .strip_prefix("claude-code/")
        .unwrap_or(router_id)
        .to_string()
}

#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    /// OAuth access token; sent as `authorization: Bearer <token>`.
    pub credential_value: String,
    pub model: String, // upstream model id (router id mapped)
    pub max_tokens: u64,
    pub api_url: String,
}

/// Why an effective config could not be built — each becomes a permanent error
/// frame whose message names the actual problem (and the fix).
#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    NotConfigured,
    ApiKeyRejected,
    /// `api_url` is set but is not an absolute http(s) URL. Carries the
    /// offending value so the error frame can show it.
    InvalidApiUrl(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotConfigured => f.write_str(
                "provider claude-code not configured: sign in with Claude Code (`claude`), \
                 or provide a Claude Pro/Max OAuth credential via the auth-credentials vault \
                 (~/.claude/.credentials.json is read as a dev fallback)",
            ),
            ConfigError::ApiKeyRejected => f.write_str(
                "provider claude-code requires a Claude Pro/Max OAuth login (oauth::claude-code); \
                 API keys belong on provider-anthropic under provider \"anthropic\"",
            ),
            ConfigError::InvalidApiUrl(u) => write!(
                f,
                "provider claude-code has an invalid endpoint url: {u:?} \
                 (must be an absolute http(s) URL)"
            ),
        }
    }
}

/// The vault may return `{ type:"oauth", access_token, ... }` or a nested
/// `{ credential: { access_token, ... } }`; tolerate both. Shared with model
/// discovery, which needs the same OAuth bearer.
pub fn extract_access_token(cred: &Value) -> Option<String> {
    cred.get("access_token")
        .and_then(Value::as_str)
        .or_else(|| {
            cred.pointer("/credential/access_token")
                .and_then(Value::as_str)
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn looks_like_api_key(cred: &Value) -> bool {
    cred.get("type").and_then(Value::as_str) == Some("api_key")
        || (cred.get("key").and_then(Value::as_str).is_some()
            && extract_access_token(cred).is_none())
}

/// Build the authenticated config from the vault credential + router resolve.
pub fn build_config(
    router_model: &str,
    effective_max_tokens: Option<u64>,
    resolved: &ProviderResolveResponse,
    credential: Option<&Value>,
) -> Result<ClaudeCodeConfig, ConfigError> {
    let cred = credential.ok_or(ConfigError::NotConfigured)?;
    if looks_like_api_key(cred) {
        return Err(ConfigError::ApiKeyRejected);
    }
    let access_token = extract_access_token(cred).ok_or(ConfigError::NotConfigured)?;

    // Trim the api_url: a stray space breaks URL parsing, surfacing only as an
    // opaque reqwest "builder error" at send.
    let api_url = match resolved.api_url.as_deref().map(str::trim) {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => DEFAULT_API_URL.to_string(),
    };
    match reqwest::Url::parse(&api_url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => {}
        _ => return Err(ConfigError::InvalidApiUrl(api_url)),
    }

    Ok(ClaudeCodeConfig {
        credential_value: access_token,
        model: upstream_model_id(router_model),
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
    use serde_json::json;

    fn resolved(api_url: Option<&str>, max_tokens: Option<u64>) -> ProviderResolveResponse {
        ProviderResolveResponse {
            configured: true,
            source: CredentialSource::Config,
            credential: None,
            api_url: api_url.map(str::to_string),
            max_tokens,
        }
    }

    fn oauth() -> Value {
        json!({ "type": "oauth", "access_token": "sk-ant-oat01-x" })
    }

    #[test]
    fn missing_credential_is_not_configured() {
        assert_eq!(
            build_config(
                "claude-code/claude-sonnet-4-6",
                None,
                &resolved(None, None),
                None
            )
            .unwrap_err(),
            ConfigError::NotConfigured
        );
    }

    #[test]
    fn api_key_credential_is_rejected() {
        let cred = json!({ "type": "api_key", "key": "sk-ant-api03-x" });
        assert_eq!(
            build_config(
                "claude-code/claude-sonnet-4-6",
                None,
                &resolved(None, None),
                Some(&cred)
            )
            .unwrap_err(),
            ConfigError::ApiKeyRejected
        );
    }

    #[test]
    fn oauth_credential_yields_token_and_upstream_model() {
        let cred = oauth();
        let cfg = build_config(
            "claude-code/claude-sonnet-4-6",
            Some(1000),
            &resolved(None, None),
            Some(&cred),
        )
        .unwrap();
        assert_eq!(cfg.credential_value, "sk-ant-oat01-x");
        assert_eq!(cfg.model, "claude-sonnet-4-6");
        assert_eq!(cfg.max_tokens, 1000);
        assert_eq!(cfg.api_url, DEFAULT_API_URL);
    }

    #[test]
    fn nested_credential_access_token_is_tolerated() {
        let cred = json!({ "credential": { "access_token": "  sk-ant-oat01-nested  " } });
        let cfg = build_config(
            "claude-code/claude-sonnet-4-6",
            None,
            &resolved(None, None),
            Some(&cred),
        )
        .unwrap();
        assert_eq!(cfg.credential_value, "sk-ant-oat01-nested");
    }

    #[test]
    fn api_url_is_trimmed_and_kept() {
        let cred = oauth();
        let cfg = build_config(
            "m",
            None,
            &resolved(Some(" https://h/v1/messages "), None),
            Some(&cred),
        )
        .unwrap();
        assert_eq!(cfg.api_url, "https://h/v1/messages");
    }

    #[test]
    fn non_http_api_url_is_rejected() {
        let cred = oauth();
        let err = build_config(
            "m",
            None,
            &resolved(Some("localhost:1234"), None),
            Some(&cred),
        )
        .unwrap_err();
        assert_eq!(err, ConfigError::InvalidApiUrl("localhost:1234".into()));
    }

    #[test]
    fn max_tokens_precedence_effective_then_configured_then_default() {
        let cred = oauth();
        let cfg = build_config("m", Some(1000), &resolved(None, Some(2000)), Some(&cred)).unwrap();
        assert_eq!(cfg.max_tokens, 1000);
        let cfg = build_config("m", None, &resolved(None, Some(2000)), Some(&cred)).unwrap();
        assert_eq!(cfg.max_tokens, 2000);
        let cfg = build_config("m", None, &resolved(None, None), Some(&cred)).unwrap();
        assert_eq!(cfg.max_tokens, DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn namespaced_router_ids_map_to_upstream() {
        assert_eq!(
            upstream_model_id("claude-code/claude-opus-4-8"),
            "claude-opus-4-8"
        );
        assert_eq!(upstream_model_id("claude-opus-4-8"), "claude-opus-4-8");
    }
}
