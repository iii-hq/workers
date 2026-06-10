//! The `router::provider::resolve` and `router::provider::update_credential`
//! iii functions. Credential precedence: stored slice (`credential` object or
//! `api_key`) → declared env var → none.
//!
//! Engine-backed coverage: tests/integration.rs (resolve precedence,
//! update_credential round-trip).
use std::sync::Arc;

use crate::types::credential::Credential;
use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::{CredentialSource, ProviderDeclaration, ProviderResolveResponse};
use futures::future::BoxFuture;
use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use crate::config::entry::{read_entry_value, write_entry_value, EntryWriteLock};
use crate::registry::store::RegistryStore;
use crate::settings::provider_slices;

/// Core resolution — shared by the resolve handler, provider::list, and chat.
pub async fn resolve_provider_config(
    iii: &III,
    declaration: &ProviderDeclaration,
) -> ProviderResolveResponse {
    let entry = read_entry_value(iii).await;
    let slice = provider_slices(&entry)
        .get(&declaration.id)
        .cloned()
        .unwrap_or(Value::Null);

    let api_url = slice
        .get("api_url")
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| {
            declaration
                .defaults
                .as_ref()
                .and_then(|d| d.api_url.clone())
        });
    let max_tokens = slice
        .get("max_tokens")
        .and_then(Value::as_u64)
        .or_else(|| declaration.defaults.as_ref().and_then(|d| d.max_tokens));

    let (credential, source) = if let Ok(cred) = serde_json::from_value::<Credential>(
        slice.get("credential").cloned().unwrap_or(Value::Null),
    ) {
        (Some(cred), CredentialSource::Config) // written by update_credential (oauth)
    } else if let Some(key) = slice
        .get("api_key")
        .and_then(Value::as_str)
        .filter(|k| !k.is_empty())
    {
        (
            Some(Credential::ApiKey {
                key: key.to_string(),
            }),
            CredentialSource::Config,
        )
    } else if let Some(env_key) = declaration
        .credential_env_var
        .as_ref()
        .and_then(|var| std::env::var(var).ok())
        .filter(|k| !k.is_empty())
    {
        (
            Some(Credential::ApiKey { key: env_key }),
            CredentialSource::Env,
        )
    } else {
        (None, CredentialSource::None)
    };

    ProviderResolveResponse {
        configured: credential.is_some(),
        source,
        credential,
        api_url,
        max_tokens,
    }
}

pub fn make_provider_resolve(
    iii: III,
    registry: Arc<RegistryStore>,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |raw: Value| {
        let (iii, registry) = (iii.clone(), registry.clone());
        Box::pin(async move {
            let id = raw
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let token = raw.get("token").and_then(Value::as_str).map(String::from);
            let record = registry
                .verify_token(&id, token.as_deref())
                .await
                .map_err(IIIError::from)?;
            let res = resolve_provider_config(&iii, &record.declaration).await;
            Ok(serde_json::to_value(res).expect("serializable response"))
        })
    }
}

/// OAuth write-back (spec § update_credential): providers never write the
/// configuration entry directly. Read-merge-write under the entry lock.
pub fn make_update_credential(
    iii: III,
    registry: Arc<RegistryStore>,
    entry_lock: EntryWriteLock,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |raw: Value| {
        let (iii, registry, entry_lock) = (iii.clone(), registry.clone(), entry_lock.clone());
        Box::pin(async move {
            let id = raw
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let token = raw.get("token").and_then(Value::as_str).map(String::from);
            registry
                .verify_token(&id, token.as_deref())
                .await
                .map_err(IIIError::from)?;
            let credential = raw.get("credential").cloned().unwrap_or(Value::Null);
            if !credential.is_object() {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    "credential object is required",
                )
                .into());
            }
            let _guard = entry_lock.lock().await;
            let mut entry = read_entry_value(&iii).await;
            if !entry.is_object() {
                entry = json!({});
            }
            let providers = entry
                .as_object_mut()
                .expect("object")
                .entry("providers")
                .or_insert_with(|| json!({}));
            let slice = providers
                .as_object_mut()
                .expect("object")
                .entry(&id)
                .or_insert_with(|| json!({}));
            slice["credential"] = credential;
            write_entry_value(&iii, entry).await?;
            Ok(json!({ "ok": true }))
        })
    }
}
