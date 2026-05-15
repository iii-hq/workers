//! Multi-process safe credential store for API keys and OAuth tokens.
//!
//! The store is split into a [`CredentialStore`] trait (storage abstraction) and
//! pure resolution helpers. The production backend writes to iii state via CAS;
//! the in-memory backend is provided for tests.

pub const SKILL_ID: &str = "auth-credentials";
pub const SKILL_MD: &str = include_str!("../skills/index.md");

pub const SUB_SKILLS: &[(&str, &str)] = &[
    (
        "auth-credentials/auth/set_token",
        include_str!("../skills/auth/set_token.md"),
    ),
    (
        "auth-credentials/auth/get_token",
        include_str!("../skills/auth/get_token.md"),
    ),
    (
        "auth-credentials/auth/delete_token",
        include_str!("../skills/auth/delete_token.md"),
    ),
    (
        "auth-credentials/auth/list_providers",
        include_str!("../skills/auth/list_providers.md"),
    ),
    (
        "auth-credentials/auth/status",
        include_str!("../skills/auth/status.md"),
    ),
];

pub mod io;
pub mod store_iii_state;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Stored credential for a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Credential {
    ApiKey {
        /// Raw API key bytes for the provider.
        key: String,
    },
    #[serde(rename = "oauth")]
    OAuth {
        /// Bearer access token returned by the provider OAuth flow.
        access_token: String,
        /// Optional refresh token when the provider issues one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_token: Option<String>,
        /// Optional Unix timestamp when the access token expires.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<i64>,
        /// OAuth scopes granted to the token.
        #[serde(default)]
        scopes: Vec<String>,
        /// Provider-specific OAuth metadata.
        #[serde(default)]
        provider_extra: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialType {
    ApiKey,
    #[serde(rename = "oauth")]
    OAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthSource {
    Stored,
    Runtime,
    Environment,
    Fallback,
}

/// Status of a provider's credential resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuthStatus {
    /// True when the worker can resolve a credential for the provider.
    pub configured: bool,
    /// Source that satisfied resolution. Omitted when no credential exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AuthSource>,
    /// Redacted display label. Never contains the full credential.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfiguredProvider {
    pub provider: String,
    pub credential_type: CredentialType,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvKeyMatch {
    pub provider: String,
    pub env_var: String,
    pub key_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub struct ProviderInput {
    /// Provider identifier, for example "anthropic" or "openai".
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub struct SetTokenInput {
    /// Provider identifier, for example "anthropic" or "openai".
    pub provider: String,
    /// Credential payload to persist for the provider.
    pub credential: Credential,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct OkOutput {
    pub ok: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub struct ListProvidersInput {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ListProvidersOutput {
    pub providers: Vec<String>,
}

/// Storage backend abstraction. Production impl writes to iii state; the
/// in-memory impl is provided here for tests.
#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn get(&self, provider: &str) -> anyhow::Result<Option<Credential>>;
    async fn set(&self, provider: &str, credential: Credential) -> anyhow::Result<()>;
    async fn clear(&self, provider: &str) -> anyhow::Result<()>;
    async fn list(&self) -> anyhow::Result<Vec<(String, Credential)>>;
}

/// In-memory credential store. Used for tests and local-only sessions.
#[derive(Debug, Clone, Default)]
pub struct InMemoryStore {
    inner: Arc<RwLock<HashMap<String, Credential>>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CredentialStore for InMemoryStore {
    async fn get(&self, provider: &str) -> anyhow::Result<Option<Credential>> {
        let g = self
            .inner
            .read()
            .map_err(|e| anyhow::anyhow!("InMemoryStore read lock poisoned: {e}"))?;
        Ok(g.get(provider).cloned())
    }

    async fn set(&self, provider: &str, credential: Credential) -> anyhow::Result<()> {
        let mut g = self
            .inner
            .write()
            .map_err(|e| anyhow::anyhow!("InMemoryStore write lock poisoned: {e}"))?;
        g.insert(provider.to_string(), credential);
        Ok(())
    }

    async fn clear(&self, provider: &str) -> anyhow::Result<()> {
        let mut g = self
            .inner
            .write()
            .map_err(|e| anyhow::anyhow!("InMemoryStore write lock poisoned: {e}"))?;
        g.remove(provider);
        Ok(())
    }

    async fn list(&self) -> anyhow::Result<Vec<(String, Credential)>> {
        let g = self
            .inner
            .read()
            .map_err(|e| anyhow::anyhow!("InMemoryStore read lock poisoned: {e}"))?;
        Ok(g.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    }
}

/// Per-provider environment variable map. Returned in stable order.
pub fn env_var_map() -> &'static [(&'static str, &'static str)] {
    &[
        ("anthropic", "ANTHROPIC_API_KEY"),
        ("openai", "OPENAI_API_KEY"),
        ("openai-codex", "OPENAI_API_KEY"),
        ("azure-openai", "AZURE_OPENAI_API_KEY"),
        ("google", "GOOGLE_API_KEY"),
        ("google-vertex", "GOOGLE_APPLICATION_CREDENTIALS"),
        ("amazon-bedrock", "AWS_ACCESS_KEY_ID"),
        ("mistral", "MISTRAL_API_KEY"),
        ("groq", "GROQ_API_KEY"),
        ("cerebras", "CEREBRAS_API_KEY"),
        ("xai", "XAI_API_KEY"),
        ("deepseek", "DEEPSEEK_API_KEY"),
        ("openrouter", "OPENROUTER_API_KEY"),
        ("vercel-ai-gateway", "VERCEL_AI_GATEWAY_API_KEY"),
        ("zai", "ZAI_API_KEY"),
        ("minimax", "MINIMAX_API_KEY"),
        ("huggingface", "HF_TOKEN"),
        ("fireworks", "FIREWORKS_API_KEY"),
        ("kimi-coding", "MOONSHOT_API_KEY"),
        ("opencode-zen", "OPENCODE_ZEN_API_KEY"),
        ("opencode-go", "OPENCODE_GO_API_KEY"),
    ]
}

/// Scan the provided environment map for present, non-empty values.
pub fn find_env_keys<F>(getter: F) -> Vec<EnvKeyMatch>
where
    F: Fn(&str) -> Option<String>,
{
    env_var_map()
        .iter()
        .filter_map(|(provider, var)| {
            let value = getter(var)?;
            if value.is_empty() {
                return None;
            }
            let prefix: String = value.chars().take(8).collect();
            Some(EnvKeyMatch {
                provider: (*provider).to_string(),
                env_var: (*var).to_string(),
                key_prefix: prefix,
            })
        })
        .collect()
}

/// Resolve a credential by source priority: stored → environment → none.
/// Pass `getter` for environment-variable lookup.
pub async fn resolve_credential<F>(
    store: &dyn CredentialStore,
    provider: &str,
    getter: F,
) -> anyhow::Result<Option<(Credential, AuthSource)>>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(c) = store.get(provider).await? {
        return Ok(Some((c, AuthSource::Stored)));
    }
    let env_var = match env_var_map()
        .iter()
        .find(|(p, _)| *p == provider)
        .map(|(_, v)| *v)
    {
        Some(v) => v,
        None => return Ok(None),
    };
    let key = match getter(env_var) {
        Some(k) if !k.is_empty() => k,
        _ => return Ok(None),
    };
    Ok(Some((Credential::ApiKey { key }, AuthSource::Environment)))
}

/// Compute auth status for a provider from a resolved credential.
pub fn status_for(resolved: Option<&(Credential, AuthSource)>) -> AuthStatus {
    match resolved {
        Some((cred, source)) => AuthStatus {
            configured: true,
            source: Some(source.clone()),
            label: Some(label_for(cred)),
        },
        None => AuthStatus {
            configured: false,
            source: None,
            label: None,
        },
    }
}

fn label_for(cred: &Credential) -> String {
    match cred {
        Credential::ApiKey { key } => {
            format!("api-key:{}…", key.chars().take(6).collect::<String>())
        }
        Credential::OAuth { .. } => "oauth".to_string(),
    }
}

fn normalize_provider(provider: &str) -> anyhow::Result<String> {
    let provider = provider.trim();
    if provider.is_empty() {
        return Err(anyhow::anyhow!("provider must be non-empty"));
    }
    Ok(provider.to_string())
}

pub async fn handle_get_token<F>(
    store: &dyn CredentialStore,
    input: ProviderInput,
    getter: F,
) -> anyhow::Result<Option<Credential>>
where
    F: Fn(&str) -> Option<String>,
{
    let provider = normalize_provider(&input.provider)?;
    Ok(resolve_credential(store, &provider, getter)
        .await?
        .map(|(credential, _source)| credential))
}

pub async fn handle_set_token(
    store: &dyn CredentialStore,
    input: SetTokenInput,
) -> anyhow::Result<OkOutput> {
    let provider = normalize_provider(&input.provider)?;
    store.set(&provider, input.credential).await?;
    Ok(OkOutput { ok: true })
}

pub async fn handle_delete_token(
    store: &dyn CredentialStore,
    input: ProviderInput,
) -> anyhow::Result<OkOutput> {
    let provider = normalize_provider(&input.provider)?;
    store.clear(&provider).await?;
    Ok(OkOutput { ok: true })
}

pub async fn handle_list_providers(
    store: &dyn CredentialStore,
    _input: ListProvidersInput,
) -> anyhow::Result<ListProvidersOutput> {
    let entries = store.list().await?;
    let mut providers: Vec<String> = entries.into_iter().map(|(provider, _)| provider).collect();
    providers.sort();
    providers.dedup();
    Ok(ListProvidersOutput { providers })
}

pub async fn handle_status<F>(
    store: &dyn CredentialStore,
    input: ProviderInput,
    getter: F,
) -> anyhow::Result<AuthStatus>
where
    F: Fn(&str) -> Option<String>,
{
    let provider = normalize_provider(&input.provider)?;
    let resolved = resolve_credential(store, &provider, getter).await?;
    Ok(status_for(resolved.as_ref()))
}

/// Register `auth::*` iii functions on the bus.
///
/// Functions registered:
/// - `auth::get_token` — payload `{ provider }`, returns the stored
///   credential or `null`
/// - `auth::set_token` — payload matches [`Credential`] with a `provider`
///   field added; returns `{ ok: true }`
/// - `auth::delete_token` — payload `{ provider }`; returns `{ ok: true }`
/// - `auth::list_providers` — returns `{ providers: [<provider>...] }`
/// - `auth::status` — payload `{ provider }`, returns an [`AuthStatus`]
///   merging stored creds and the process env
///
/// `store` is the backend the handlers read/write through. Tests pass an
/// [`InMemoryStore`]; production callers pass an iii-state-backed impl.
pub async fn register_with_iii(
    iii: &iii_sdk::III,
    store: std::sync::Arc<dyn CredentialStore>,
) -> anyhow::Result<AuthFunctionRefs> {
    use iii_sdk::{IIIError, RegisterFunction};

    let store_get = store.clone();
    let get_token = iii.register_function(
        RegisterFunction::new_async("auth::get_token", move |input: ProviderInput| {
            let store = store_get.clone();
            async move {
                handle_get_token(&*store, input, |var| std::env::var(var).ok())
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))
            }
        })
        .description("Fetch the stored or environment credential for a provider."),
    );

    let store_set = store.clone();
    let set_token = iii.register_function(
        RegisterFunction::new_async("auth::set_token", move |input: SetTokenInput| {
            let store = store_set.clone();
            async move {
                handle_set_token(&*store, input)
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))
            }
        })
        .description("Persist a credential for a provider."),
    );

    let store_del = store.clone();
    let delete_token = iii.register_function(
        RegisterFunction::new_async("auth::delete_token", move |input: ProviderInput| {
            let store = store_del.clone();
            async move {
                handle_delete_token(&*store, input)
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))
            }
        })
        .description("Remove the stored credential for a provider."),
    );

    let store_list = store.clone();
    let list_providers = iii.register_function(
        RegisterFunction::new_async("auth::list_providers", move |input: ListProvidersInput| {
            let store = store_list.clone();
            async move {
                handle_list_providers(&*store, input)
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))
            }
        })
        .description("List every provider with a stored credential."),
    );

    let store_status = store.clone();
    let status = iii.register_function(
        RegisterFunction::new_async("auth::status", move |input: ProviderInput| {
            let store = store_status.clone();
            async move {
                handle_status(&*store, input, |var| std::env::var(var).ok())
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))
            }
        })
        .description("Report stored vs. environment credential status for a provider."),
    );

    Ok(AuthFunctionRefs {
        get_token,
        set_token,
        delete_token,
        list_providers,
        status,
    })
}

/// Handles returned by [`register_with_iii`]. Calling `unregister_all`
/// removes every function from the bus.
pub struct AuthFunctionRefs {
    pub get_token: iii_sdk::FunctionRef,
    pub set_token: iii_sdk::FunctionRef,
    pub delete_token: iii_sdk::FunctionRef,
    pub list_providers: iii_sdk::FunctionRef,
    pub status: iii_sdk::FunctionRef,
}

impl AuthFunctionRefs {
    pub fn unregister_all(self) {
        for r in [
            self.get_token,
            self.set_token,
            self.delete_token,
            self.list_providers,
            self.status,
        ] {
            r.unregister();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_roundtrip() -> anyhow::Result<()> {
        let s = InMemoryStore::new();
        s.set(
            "anthropic",
            Credential::ApiKey {
                key: "sk-ant-xxx".into(),
            },
        )
        .await?;
        let got = s.get("anthropic").await?.unwrap();
        assert_eq!(
            got,
            Credential::ApiKey {
                key: "sk-ant-xxx".into()
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn clear_removes() -> anyhow::Result<()> {
        let s = InMemoryStore::new();
        s.set("openai", Credential::ApiKey { key: "x".into() })
            .await?;
        assert!(s.get("openai").await?.is_some());
        s.clear("openai").await?;
        assert!(s.get("openai").await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn list_returns_all() -> anyhow::Result<()> {
        let s = InMemoryStore::new();
        s.set("anthropic", Credential::ApiKey { key: "a".into() })
            .await?;
        s.set("openai", Credential::ApiKey { key: "b".into() })
            .await?;
        let listed = s.list().await?;
        assert_eq!(listed.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn resolve_prefers_stored_over_env() -> anyhow::Result<()> {
        let s = InMemoryStore::new();
        s.set(
            "anthropic",
            Credential::ApiKey {
                key: "stored".into(),
            },
        )
        .await?;
        let result =
            resolve_credential(&s, "anthropic", |_| Some("env-fallback".to_string())).await?;
        let (cred, source) = result.unwrap();
        assert!(matches!(source, AuthSource::Stored));
        assert_eq!(
            cred,
            Credential::ApiKey {
                key: "stored".into()
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn resolve_falls_back_to_env() -> anyhow::Result<()> {
        let s = InMemoryStore::new();
        let result = resolve_credential(&s, "openai", |var| {
            if var == "OPENAI_API_KEY" {
                Some("from-env".to_string())
            } else {
                None
            }
        })
        .await?;
        let (cred, source) = result.unwrap();
        assert!(matches!(source, AuthSource::Environment));
        assert_eq!(
            cred,
            Credential::ApiKey {
                key: "from-env".into()
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn resolve_returns_none_when_unknown() -> anyhow::Result<()> {
        let s = InMemoryStore::new();
        let result = resolve_credential(&s, "nope", |_| None).await?;
        assert!(result.is_none());
        Ok(())
    }

    #[test]
    fn find_env_keys_skips_empty() {
        let matches = find_env_keys(|var| match var {
            "ANTHROPIC_API_KEY" => Some("sk-ant-actual".to_string()),
            "OPENAI_API_KEY" => Some(String::new()),
            _ => None,
        });
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].provider, "anthropic");
        assert_eq!(matches[0].key_prefix, "sk-ant-a");
    }

    #[test]
    fn env_var_map_covers_known_providers() {
        let providers: Vec<&&str> = env_var_map().iter().map(|(p, _)| p).collect();
        assert!(providers.contains(&&"anthropic"));
        assert!(providers.contains(&&"openai"));
        assert!(providers.contains(&&"google"));
    }

    #[tokio::test]
    async fn list_provider_handler_returns_sorted_names_only() -> anyhow::Result<()> {
        let s = InMemoryStore::new();
        s.set("openai", Credential::ApiKey { key: "b".into() })
            .await?;
        s.set("anthropic", Credential::ApiKey { key: "a".into() })
            .await?;

        let out = handle_list_providers(&s, ListProvidersInput {}).await?;
        assert_eq!(out.providers, vec!["anthropic", "openai"]);
        Ok(())
    }

    #[tokio::test]
    async fn provider_handlers_reject_blank_provider() {
        let s = InMemoryStore::new();
        let err = handle_get_token(
            &s,
            ProviderInput {
                provider: " ".into(),
            },
            |_| None,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("provider must be non-empty"));
    }

    #[test]
    fn auth_source_serializes_snake_case() -> anyhow::Result<()> {
        let value = serde_json::to_value(AuthSource::Environment)?;
        assert_eq!(value, serde_json::json!("environment"));
        Ok(())
    }
}
