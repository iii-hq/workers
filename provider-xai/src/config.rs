//! Effective per-request config: credential + url + max_tokens.
//! Precedence for max_tokens: router-resolved effective budget
//! (`ProviderStreamInput.max_output_tokens`) → the operator's configured
//! `max_tokens` (from resolve) → the worker default.
use llm_router::types::credential::Credential;
use llm_router::types::router::ProviderResolveResponse;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_API_URL: &str = "https://api.x.ai/v1/chat/completions";
pub const DEFAULT_MAX_TOKENS: u64 = 8192;

/// The xAI server-side tools we support on the `/v1/responses` path. An enum
/// so the config schema (and thus the console + deserialize path) rejects an
/// unknown tool name before it can be stored or forwarded to the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    XSearch,
    WebSearch,
    CodeInterpreter,
    CollectionsSearch,
}

impl ToolSource {
    /// The `type` string sent in the `/v1/responses` `tools` array.
    pub fn as_type(self) -> &'static str {
        match self {
            ToolSource::XSearch => "x_search",
            ToolSource::WebSearch => "web_search",
            ToolSource::CodeInterpreter => "code_interpreter",
            ToolSource::CollectionsSearch => "collections_search",
        }
    }
}

/// Operator-tunable worker config, registered with the `configuration` worker
/// so it renders as an editable item in the console configuration sidebar.
/// Distinct from the per-request `XaiConfig` (credentials/url/max_tokens come
/// from the llm-router resolve step); this only governs xAI's Agent Tools
/// (server-side `x_search` / `web_search` on the `/v1/responses` API).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WorkerConfig {
    /// Enable xAI Agent Tools (live X / web search via the /v1/responses API).
    /// Off = the provider is a plain Chat Completions inference wrapper.
    pub tools_enabled: bool,
    /// Server-side tools offered when enabled.
    pub tool_sources: Vec<ToolSource>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            tools_enabled: false,
            tool_sources: vec![ToolSource::XSearch, ToolSource::WebSearch],
        }
    }
}

impl WorkerConfig {
    pub fn json_schema() -> Value {
        let root = schemars::gen::SchemaGenerator::default().into_root_schema_for::<WorkerConfig>();
        serde_json::to_value(root).expect("config schema serializes")
    }
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("config serializes")
    }
    pub fn from_json(value: &Value) -> Result<WorkerConfig, serde_json::Error> {
        serde_json::from_value(value.clone())
    }
}

#[derive(Debug, Clone)]
pub struct XaiConfig {
    pub credential_value: String,
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
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotConfigured => f.write_str(
                "provider xai not configured (no api_key in the llm-router entry \
                 and XAI_API_KEY unset)",
            ),
            ConfigError::InvalidApiUrl(u) => write!(
                f,
                "provider xai has an invalid endpoint url: {u:?} \
                 (must be an absolute http(s) URL)"
            ),
        }
    }
}

/// The single Credential → bearer secret mapping; streaming and discovery
/// must agree on it. xAI takes `Authorization: Bearer` for both shapes.
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
) -> Result<XaiConfig, ConfigError> {
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
    Ok(XaiConfig {
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

    #[test]
    fn worker_config_rejects_unknown_tool_and_maps_types() {
        // valid enum values round-trip and carry the right API type strings
        let cfg = WorkerConfig::from_json(&serde_json::json!({ "tools_enabled": true,
                "tool_sources": ["x_search", "code_interpreter"] }))
        .unwrap();
        assert!(cfg.tools_enabled);
        assert_eq!(cfg.tool_sources[0].as_type(), "x_search");
        assert_eq!(cfg.tool_sources[1].as_type(), "code_interpreter");
        // an unknown tool name is rejected at the deserialize boundary
        assert!(
            WorkerConfig::from_json(&serde_json::json!({ "tool_sources": ["not_a_tool"] }))
                .is_err()
        );
    }

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
        assert_eq!(cfg.credential_value, "at");
    }
}
