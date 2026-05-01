//! Multi-process safe credential store for API keys and OAuth tokens.
//!
//! The store is split into a [`CredentialStore`] trait (storage abstraction) and
//! pure resolution helpers. The production backend writes to iii state via CAS;
//! the in-memory backend is provided for tests.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Stored credential for a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Credential {
    ApiKey {
        key: String,
    },
    OAuth {
        access_token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<i64>,
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(default)]
        provider_extra: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialType {
    ApiKey,
    OAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthSource {
    Stored,
    Runtime,
    Environment,
    Fallback,
}

/// Status of a provider's credential resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthStatus {
    pub configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AuthSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredProvider {
    pub provider: String,
    pub credential_type: CredentialType,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvKeyMatch {
    pub provider: String,
    pub env_var: String,
    pub key_prefix: String,
}

/// Storage backend abstraction. Production impl writes to iii state; the
/// in-memory impl is provided here for tests.
#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn get(&self, provider: &str) -> Option<Credential>;
    async fn set(&self, provider: &str, credential: Credential);
    async fn clear(&self, provider: &str);
    async fn list(&self) -> Vec<(String, Credential)>;
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
    async fn get(&self, provider: &str) -> Option<Credential> {
        self.inner.read().ok()?.get(provider).cloned()
    }

    async fn set(&self, provider: &str, credential: Credential) {
        if let Ok(mut g) = self.inner.write() {
            g.insert(provider.to_string(), credential);
        }
    }

    async fn clear(&self, provider: &str) {
        if let Ok(mut g) = self.inner.write() {
            g.remove(provider);
        }
    }

    async fn list(&self) -> Vec<(String, Credential)> {
        self.inner
            .read()
            .map(|g| g.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
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
) -> Option<(Credential, AuthSource)>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(c) = store.get(provider).await {
        return Some((c, AuthSource::Stored));
    }
    let env_var = env_var_map()
        .iter()
        .find(|(p, _)| *p == provider)
        .map(|(_, v)| *v)?;
    let key = getter(env_var)?;
    if key.is_empty() {
        return None;
    }
    Some((Credential::ApiKey { key }, AuthSource::Environment))
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
    use iii_sdk::{IIIError, RegisterFunctionMessage};
    use serde_json::{json, Value};

    let store_get = store.clone();
    let get_token = iii.register_function((
        RegisterFunctionMessage::with_id("auth::get_token".to_string())
            .with_description("Fetch the stored credential for a provider.".into()),
        move |payload: Value| {
            let store = store_get.clone();
            async move {
                let provider = payload
                    .get("provider")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing required field: provider".into()))?
                    .to_string();
                // P5: providers call `auth::get_token` as their single
                // credential entry point. Resolve stored-then-env so callers
                // never re-read env directly. Returning `null` means the
                // provider has neither a stored credential nor an env match.
                let resolved =
                    resolve_credential(&*store, &provider, |var| std::env::var(var).ok()).await;
                let cred = resolved.map(|(c, _source)| c);
                serde_json::to_value(cred).map_err(|e| IIIError::Handler(e.to_string()))
            }
        },
    ));

    let store_set = store.clone();
    let set_token = iii.register_function((
        RegisterFunctionMessage::with_id("auth::set_token".to_string())
            .with_description("Persist a credential for a provider.".into()),
        move |payload: Value| {
            let store = store_set.clone();
            async move {
                let provider = payload
                    .get("provider")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing required field: provider".into()))?
                    .to_string();
                let cred_value = payload.get("credential").cloned().ok_or_else(|| {
                    IIIError::Handler("missing required field: credential".into())
                })?;
                let cred: Credential = serde_json::from_value(cred_value)
                    .map_err(|e| IIIError::Handler(format!("invalid credential: {e}")))?;
                store.set(&provider, cred).await;
                Ok(json!({ "ok": true }))
            }
        },
    ));

    let store_del = store.clone();
    let delete_token = iii.register_function((
        RegisterFunctionMessage::with_id("auth::delete_token".to_string())
            .with_description("Remove the stored credential for a provider.".into()),
        move |payload: Value| {
            let store = store_del.clone();
            async move {
                let provider = payload
                    .get("provider")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing required field: provider".into()))?
                    .to_string();
                store.clear(&provider).await;
                Ok(json!({ "ok": true }))
            }
        },
    ));

    let store_list = store.clone();
    let list_providers = iii.register_function((
        RegisterFunctionMessage::with_id("auth::list_providers".to_string())
            .with_description("List every provider with a stored credential.".into()),
        move |_payload: Value| {
            let store = store_list.clone();
            async move {
                let entries = store.list().await;
                let providers: Vec<String> = entries.into_iter().map(|(p, _)| p).collect();
                Ok(json!({ "providers": providers }))
            }
        },
    ));

    let store_status = store.clone();
    let status = iii.register_function((
        RegisterFunctionMessage::with_id("auth::status".to_string())
            .with_description("Report stored vs. env credential status for a provider.".into()),
        move |payload: Value| {
            let store = store_status.clone();
            async move {
                let provider = payload
                    .get("provider")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing required field: provider".into()))?
                    .to_string();
                let resolved =
                    resolve_credential(&*store, &provider, |var| std::env::var(var).ok()).await;
                let st = status_for(resolved.as_ref());
                serde_json::to_value(st).map_err(|e| IIIError::Handler(e.to_string()))
            }
        },
    ));

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
    async fn in_memory_roundtrip() {
        let s = InMemoryStore::new();
        s.set(
            "anthropic",
            Credential::ApiKey {
                key: "sk-ant-xxx".into(),
            },
        )
        .await;
        let got = s.get("anthropic").await.unwrap();
        assert_eq!(
            got,
            Credential::ApiKey {
                key: "sk-ant-xxx".into()
            }
        );
    }

    #[tokio::test]
    async fn clear_removes() {
        let s = InMemoryStore::new();
        s.set("openai", Credential::ApiKey { key: "x".into() })
            .await;
        assert!(s.get("openai").await.is_some());
        s.clear("openai").await;
        assert!(s.get("openai").await.is_none());
    }

    #[tokio::test]
    async fn list_returns_all() {
        let s = InMemoryStore::new();
        s.set("anthropic", Credential::ApiKey { key: "a".into() })
            .await;
        s.set("openai", Credential::ApiKey { key: "b".into() })
            .await;
        let listed = s.list().await;
        assert_eq!(listed.len(), 2);
    }

    #[tokio::test]
    async fn resolve_prefers_stored_over_env() {
        let s = InMemoryStore::new();
        s.set(
            "anthropic",
            Credential::ApiKey {
                key: "stored".into(),
            },
        )
        .await;
        let result =
            resolve_credential(&s, "anthropic", |_| Some("env-fallback".to_string())).await;
        let (cred, source) = result.unwrap();
        assert!(matches!(source, AuthSource::Stored));
        assert_eq!(
            cred,
            Credential::ApiKey {
                key: "stored".into()
            }
        );
    }

    #[tokio::test]
    async fn resolve_falls_back_to_env() {
        let s = InMemoryStore::new();
        let result = resolve_credential(&s, "openai", |var| {
            if var == "OPENAI_API_KEY" {
                Some("from-env".to_string())
            } else {
                None
            }
        })
        .await;
        let (cred, source) = result.unwrap();
        assert!(matches!(source, AuthSource::Environment));
        assert_eq!(
            cred,
            Credential::ApiKey {
                key: "from-env".into()
            }
        );
    }

    #[tokio::test]
    async fn resolve_returns_none_when_unknown() {
        let s = InMemoryStore::new();
        let result = resolve_credential(&s, "nope", |_| None).await;
        assert!(result.is_none());
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
}
