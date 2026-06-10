//! The `router::provider::resolve` and `router::provider::update_credential`
//! iii functions. Credential precedence: stored slice (`credential` object or
//! `api_key`) → declared env var → none.
use std::sync::Arc;

use crate::types::credential::Credential;
use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::{CredentialSource, ProviderDeclaration, ProviderResolveResponse};
use serde_json::{json, Value};

use crate::bus::{handler, Bus, BusError, Handler};
use crate::config::entry::{read_entry_value, write_entry_value, EntryWriteLock};
use crate::registry::store::RegistryStore;
use crate::settings::provider_slices;

/// Core resolution — shared by the resolve handler, provider::list, and chat.
pub async fn resolve_provider_config(
    bus: &Arc<dyn Bus>,
    declaration: &ProviderDeclaration,
) -> ProviderResolveResponse {
    let entry = read_entry_value(bus).await;
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

pub fn make_provider_resolve(bus: Arc<dyn Bus>, registry: Arc<RegistryStore>) -> Handler {
    handler(move |raw: Value| {
        let (bus, registry) = (bus.clone(), registry.clone());
        async move {
            let id = raw
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let token = raw.get("token").and_then(Value::as_str).map(String::from);
            let record = registry
                .verify_token(&id, token.as_deref())
                .await
                .map_err(BusError::from)?;
            let res = resolve_provider_config(&bus, &record.declaration).await;
            Ok(serde_json::to_value(res).expect("serializable response"))
        }
    })
}

/// OAuth write-back (spec § update_credential): providers never write the
/// configuration entry directly. Read-merge-write under the entry lock.
pub fn make_update_credential(
    bus: Arc<dyn Bus>,
    registry: Arc<RegistryStore>,
    entry_lock: EntryWriteLock,
) -> Handler {
    handler(move |raw: Value| {
        let (bus, registry, entry_lock) = (bus.clone(), registry.clone(), entry_lock.clone());
        async move {
            let id = raw
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let token = raw.get("token").and_then(Value::as_str).map(String::from);
            registry
                .verify_token(&id, token.as_deref())
                .await
                .map_err(BusError::from)?;
            let credential = raw.get("credential").cloned().unwrap_or(Value::Null);
            if !credential.is_object() {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    "credential object is required",
                )
                .into());
            }
            let _guard = entry_lock.lock().await;
            let mut entry = read_entry_value(&bus).await;
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
            write_entry_value(&bus, entry).await?;
            Ok(json!({ "ok": true }))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::store::CatalogStore;
    use crate::config::entry::EntryWriteLock;
    use crate::registry::register::make_provider_register;
    use crate::registry::store::RegistryStore;
    use crate::testkit::fake_bus::FakeBus;
    use crate::triggers::TriggerEmitter;
    use serde_json::json;
    use std::sync::Arc;

    async fn harness() -> (Arc<FakeBus>, Arc<RegistryStore>, String) {
        let bus = FakeBus::new();
        let registry = Arc::new(RegistryStore::new(bus.clone()));
        let catalog = Arc::new(CatalogStore::new(bus.clone()));
        let emitter = TriggerEmitter::install(bus.clone() as Arc<dyn crate::bus::Bus>, &*bus);
        let register = make_provider_register(
            bus.clone(),
            registry.clone(),
            catalog,
            emitter,
            EntryWriteLock::default(),
        );
        let res = register(json!({
            "id": "anthropic",
            "credential_env_var": "TEST_RESOLVE_KEY",
            "defaults": { "api_url": "https://api.anthropic.com/v1/messages", "max_tokens": 8192 }
        }))
        .await
        .unwrap();
        std::env::remove_var("TEST_RESOLVE_KEY");
        (
            bus,
            registry,
            res["registration_token"].as_str().unwrap().to_string(),
        )
    }

    #[tokio::test]
    async fn precedence_config_env_none_and_token_gate() {
        let (bus, registry, token) = harness().await;
        let resolve = make_provider_resolve(bus.clone(), registry.clone());

        // none + declared defaults
        let res = resolve(json!({ "id": "anthropic", "token": token }))
            .await
            .unwrap();
        assert_eq!(res["configured"], false);
        assert_eq!(res["source"], "none");
        assert_eq!(res["api_url"], "https://api.anthropic.com/v1/messages");
        assert_eq!(res["max_tokens"], 8192);

        // env fallback
        std::env::set_var("TEST_RESOLVE_KEY", "sk-env");
        let res = resolve(json!({ "id": "anthropic", "token": token }))
            .await
            .unwrap();
        assert_eq!(res["source"], "env");
        assert_eq!(res["credential"]["key"], "sk-env");

        // stored slice wins over env; slice max_tokens overrides the default
        bus.trigger("configuration::set", json!({ "id": "llm-router", "value": { "providers": { "anthropic": { "api_key": "sk-stored", "max_tokens": 4096 } } } }), None).await.unwrap();
        let res = resolve(json!({ "id": "anthropic", "token": token }))
            .await
            .unwrap();
        assert_eq!(res["source"], "config");
        assert_eq!(res["credential"]["key"], "sk-stored");
        assert_eq!(res["max_tokens"], 4096);
        std::env::remove_var("TEST_RESOLVE_KEY");

        // token gate
        let err = resolve(json!({ "id": "anthropic", "token": "wrong" }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Some("router/registration_rejected"));
    }

    #[tokio::test]
    async fn update_credential_persists_oauth_into_the_slice() {
        let (bus, registry, token) = harness().await;
        let update =
            make_update_credential(bus.clone(), registry.clone(), EntryWriteLock::default());
        let resolve = make_provider_resolve(bus, registry);
        let res = update(json!({
            "id": "anthropic", "token": token,
            "credential": { "type": "oauth", "access_token": "at-1", "expires_at": 99 }
        }))
        .await
        .unwrap();
        assert_eq!(res, json!({ "ok": true }));
        let resolved = resolve(json!({ "id": "anthropic", "token": token }))
            .await
            .unwrap();
        assert_eq!(resolved["source"], "config");
        assert_eq!(resolved["credential"]["access_token"], "at-1");
    }
}
