//! The `router::provider::resolve` and `router::provider::update_credential`
//! iii functions. Credential precedence: stored slice (`credential` object or
//! `api_key`) → declared env var → none.
//!
//! Engine-backed coverage: tests/integration.rs (resolve precedence,
//! update_credential round-trip).
use std::sync::Arc;

use crate::types::credential::Credential;
use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::{
    CredentialSource, ProviderDeclaration, ProviderResolveRequest, ProviderResolveResponse,
    UpdateCredentialRequest, UpdateCredentialResponse,
};
use futures::future::BoxFuture;
use iii_sdk::{errors::Error, IIIClient};
use serde_json::{json, Value};

use crate::config::entry::{read_entry_value, write_entry_value, EntryWriteLock};
use crate::config::state::{apply_config, snapshot, ConfigCell, ConfigSnapshot};
use crate::registry::store::RegistryStore;

/// Core resolution — shared by the resolve handler, provider::list, and chat.
pub fn resolve_provider_config(
    config: &ConfigSnapshot,
    declaration: &ProviderDeclaration,
) -> ProviderResolveResponse {
    let slice = config
        .provider_slice(&declaration.id)
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
    config: ConfigCell,
    registry: Arc<RegistryStore>,
) -> impl Fn(ProviderResolveRequest) -> BoxFuture<'static, Result<ProviderResolveResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |req: ProviderResolveRequest| {
        let (config, registry) = (config.clone(), registry.clone());
        Box::pin(async move {
            let record = registry
                .verify_token(&req.id, req.token.as_deref())
                .await
                .map_err(Error::from)?;
            let config = snapshot(&config);
            Ok(resolve_provider_config(&config, &record.declaration))
        })
    }
}

/// OAuth write-back (spec § update_credential): providers never write the
/// configuration entry directly. Read-merge-write under the entry lock.
pub fn make_update_credential(
    iii: IIIClient,
    registry: Arc<RegistryStore>,
    config: ConfigCell,
    entry_lock: EntryWriteLock,
) -> impl Fn(UpdateCredentialRequest) -> BoxFuture<'static, Result<UpdateCredentialResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |req: UpdateCredentialRequest| {
        let (iii, registry, config, entry_lock) = (
            iii.clone(),
            registry.clone(),
            config.clone(),
            entry_lock.clone(),
        );
        Box::pin(async move {
            registry
                .verify_token(&req.id, req.token.as_deref())
                .await
                .map_err(Error::from)?;
            let credential = req.credential;
            if !credential.is_object() {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    "credential object is required",
                )
                .into());
            }
            let _guard = entry_lock.lock().await;
            let mut entry = read_entry_value(&iii).await?;
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
                .entry(&req.id)
                .or_insert_with(|| json!({}));
            slice["credential"] = credential;
            write_entry_value(&iii, entry.clone()).await?;
            // Make this worker-originated write visible immediately; the
            // asynchronous configuration trigger will subsequently re-fetch
            // the same authoritative value and drive model discovery.
            apply_config(&config, entry);
            Ok(UpdateCredentialResponse { ok: true })
        })
    }
}
