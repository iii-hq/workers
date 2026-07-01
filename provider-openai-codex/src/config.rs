//! Effective per-request config for the Codex Responses backend: OAuth access
//! token + ChatGPT account id + url + max_tokens. The credential comes from the
//! `auth-credentials` vault (`auth::get_token`); the api_url/max_tokens come
//! from the router's `resolve`. This provider is OAuth-only — API keys belong
//! on `provider-openai`.
use crate::curated::upstream_model_id;
use llm_router::types::router::ProviderResolveResponse;
use serde_json::Value;

/// Codex Responses endpoint (ChatGPT subscription backend).
pub const DEFAULT_API_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
pub const DEFAULT_MAX_TOKENS: u64 = 8192;

#[derive(Debug, Clone)]
pub struct CodexConfig {
    pub access_token: String,
    pub account_id: String,
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
    MissingAccountId,
    InvalidApiUrl(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotConfigured => f.write_str(
                "provider openai-codex not configured: sign in with ChatGPT in the console \
                 (Configuration → providers), or import an existing ~/.codex/auth.json",
            ),
            ConfigError::ApiKeyRejected => f.write_str(
                "provider openai-codex requires a ChatGPT OAuth login (oauth::openai-codex); \
                 API keys belong on provider-openai under provider \"openai\"",
            ),
            ConfigError::MissingAccountId => f.write_str(
                "provider openai-codex: missing ChatGPT account id — sign in again with \
                 oauth::openai-codex",
            ),
            ConfigError::InvalidApiUrl(u) => write!(
                f,
                "provider openai-codex has an invalid endpoint url: {u:?} \
                 (must be an absolute http(s) URL)"
            ),
        }
    }
}

/// The vault may return `{ type:"oauth", access_token, provider_extra:{...} }`
/// or a nested `{ credential: { access_token, ... } }`; tolerate both.
fn extract_access_token(cred: &Value) -> Option<String> {
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

fn extract_account_id(cred: &Value, access_token: &str) -> Option<String> {
    cred.get("account_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            cred.pointer("/provider_extra/account_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            cred.pointer("/credential/provider_extra/account_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| crate::auth::account_id_from_access_token(access_token))
}

/// Build the effective config from the vault credential + router resolve.
pub fn build_config(
    router_model: &str,
    effective_max_tokens: Option<u64>,
    resolved: &ProviderResolveResponse,
    credential: Option<&Value>,
) -> Result<CodexConfig, ConfigError> {
    let cred = credential.ok_or(ConfigError::NotConfigured)?;
    if looks_like_api_key(cred) {
        return Err(ConfigError::ApiKeyRejected);
    }
    let access_token = extract_access_token(cred).ok_or(ConfigError::NotConfigured)?;
    let account_id =
        extract_account_id(cred, &access_token).ok_or(ConfigError::MissingAccountId)?;

    let api_url = match resolved.api_url.as_deref().map(str::trim) {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => DEFAULT_API_URL.to_string(),
    };
    match reqwest::Url::parse(&api_url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => {}
        _ => return Err(ConfigError::InvalidApiUrl(api_url)),
    }

    Ok(CodexConfig {
        access_token,
        account_id,
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

    #[test]
    fn missing_credential_is_not_configured() {
        assert_eq!(
            build_config("codex/gpt-5.5", None, &resolved(None, None), None).unwrap_err(),
            ConfigError::NotConfigured
        );
    }

    #[test]
    fn api_key_credential_is_rejected() {
        let cred = json!({ "type": "api_key", "key": "sk-x" });
        assert_eq!(
            build_config("codex/gpt-5.5", None, &resolved(None, None), Some(&cred)).unwrap_err(),
            ConfigError::ApiKeyRejected
        );
    }

    #[test]
    fn oauth_credential_yields_token_account_and_upstream_model() {
        let cred = json!({
            "type": "oauth",
            "access_token": "at",
            "provider_extra": { "account_id": "acc-1" }
        });
        let cfg = build_config(
            "codex/gpt-5.5",
            Some(1000),
            &resolved(None, None),
            Some(&cred),
        )
        .unwrap();
        assert_eq!(cfg.access_token, "at");
        assert_eq!(cfg.account_id, "acc-1");
        assert_eq!(cfg.model, "gpt-5.5");
        assert_eq!(cfg.max_tokens, 1000);
        assert_eq!(cfg.api_url, DEFAULT_API_URL);
    }

    #[test]
    fn missing_account_id_is_flagged() {
        let cred = json!({ "type": "oauth", "access_token": "no-account-jwt" });
        assert_eq!(
            build_config("codex/gpt-5.5", None, &resolved(None, None), Some(&cred)).unwrap_err(),
            ConfigError::MissingAccountId
        );
    }
}
