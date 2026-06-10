//! The `router::provider::register` iii function (spec § register,
//! § Registration lifecycle): validate → token-gated upsert → entry-schema
//! re-compose (under the entry write lock) → static models reconcile → emits.
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::{ProviderDeclaration, ProviderRegisterResponse};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::bus::{handler, Bus, BusError, Handler};
use crate::catalog::store::CatalogStore;
use crate::config::entry::{register_entry, EntryWriteLock};
use crate::config::schema::{default_provider_schema, validate_custom_schema};
use crate::registry::store::RegistryStore;
use crate::triggers::TriggerEmitter;

#[derive(Deserialize)]
struct RegisterInput {
    #[serde(flatten)]
    declaration: ProviderDeclaration,
    token: Option<String>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn make_provider_register(
    bus: Arc<dyn Bus>,
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
    emitter: TriggerEmitter,
    entry_lock: EntryWriteLock,
) -> Handler {
    handler(move |raw: Value| {
        let (bus, registry, catalog, emitter, entry_lock) = (
            bus.clone(),
            registry.clone(),
            catalog.clone(),
            emitter.clone(),
            entry_lock.clone(),
        );
        async move {
            let input: RegisterInput = serde_json::from_value(raw).map_err(|e| {
                BusError::from(RouterError::new(RouterCode::InvalidRequest, e.to_string()))
            })?;
            let declaration = input.declaration;

            if !valid_id(&declaration.id) {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    format!("invalid provider id: {}", declaration.id),
                )
                .into());
            }
            if let Some(schema) = &declaration.config_schema {
                validate_custom_schema(schema).map_err(BusError::from)?;
            }
            if let Some(models) = &declaration.models {
                for m in models {
                    if m.provider != declaration.id {
                        return Err(RouterError::new(
                            RouterCode::InvalidRequest,
                            format!(
                                "static model {} declares provider {}, expected {}",
                                m.id, m.provider, declaration.id
                            ),
                        )
                        .into());
                    }
                }
            }

            let worker_id = declaration.worker_id.clone();
            let static_models = declaration.models.clone();
            let id = declaration.id.clone();
            let (_, token) = registry
                .upsert(declaration, worker_id, input.token)
                .await
                .map_err(BusError::from)?;

            // Re-compose the entry schema from every registered declaration —
            // under the entry write lock so concurrent boots compose.
            {
                let _guard = entry_lock.lock().await;
                let mut provider_schemas = BTreeMap::new();
                for rec in registry.list().await {
                    let schema = rec.declaration.config_schema.clone().unwrap_or_else(|| {
                        default_provider_schema(
                            &serde_json::to_value(rec.declaration.defaults.clone())
                                .unwrap_or(Value::Null),
                        )
                    });
                    provider_schemas.insert(rec.declaration.id.clone(), schema);
                }
                register_entry(&bus, &provider_schemas).await?;
            }

            emitter
                .emit(
                    "router::provider::changed",
                    json!({ "provider": id, "op": "register" }),
                )
                .await;

            // Static catalog slice: reconciled at registration (spec § register).
            if let Some(models) = static_models {
                if !models.is_empty() {
                    let count = models.len();
                    catalog.set_slice(&id, models).await?;
                    emitter
                        .emit(
                            "router::models::changed",
                            json!({ "provider": id, "count": count }),
                        )
                        .await;
                }
            }

            Ok(serde_json::to_value(ProviderRegisterResponse {
                ok: true,
                id,
                registration_token: token,
            })
            .expect("serializable response"))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::store::CatalogStore;
    use crate::registry::store::RegistryStore;
    use crate::testkit::fake_bus::FakeBus;
    use crate::triggers::TriggerEmitter;
    use serde_json::json;
    use std::sync::Arc;

    struct H {
        bus: Arc<FakeBus>,
        catalog: Arc<CatalogStore>,
        handler: crate::bus::Handler,
    }

    fn harness() -> H {
        let bus = FakeBus::new();
        let registry = Arc::new(RegistryStore::new(bus.clone()));
        let catalog = Arc::new(CatalogStore::new(bus.clone()));
        let emitter = TriggerEmitter::install(bus.clone() as Arc<dyn crate::bus::Bus>, &*bus);
        let handler = make_provider_register(
            bus.clone(),
            registry,
            catalog.clone(),
            emitter,
            crate::config::entry::EntryWriteLock::default(),
        );
        H {
            bus,
            catalog,
            handler,
        }
    }

    fn declaration() -> serde_json::Value {
        json!({
            "id": "anthropic",
            "display_name": "Anthropic",
            "credential_env_var": "ANTHROPIC_API_KEY",
            "defaults": { "api_url": "https://api.anthropic.com/v1/messages", "max_tokens": 8192 },
            "supports_model_listing": true,
            "worker_id": "w1",
            "models": [{ "id": "claude-sonnet-4", "provider": "anthropic", "context_window": 200000, "max_output_tokens": 64000 }]
        })
    }

    #[tokio::test]
    async fn registers_composes_schema_reconciles_static_slice_returns_token() {
        let h = harness();
        let res = (h.handler)(declaration()).await.unwrap();
        assert_eq!(res["ok"], true);
        assert!(res["registration_token"].as_str().unwrap().len() >= 16);
        assert_eq!(h.catalog.slice("anthropic").await.len(), 1);
        let entries = h.bus.config_entries.lock().unwrap();
        let schema = &entries.get("llm-router").unwrap().schema;
        assert!(schema["properties"]["providers"]["properties"]
            .get("anthropic")
            .is_some());
    }

    #[tokio::test]
    async fn rejects_bad_ids_wrong_provider_models_and_takeover() {
        let h = harness();
        let err = (h.handler)(json!({ "id": "Bad Id!" })).await.unwrap_err();
        assert_eq!(err.code(), Some("router/invalid_request"));
        let mut wrong = declaration();
        wrong["models"][0]["provider"] = json!("other");
        assert_eq!(
            (h.handler)(wrong).await.unwrap_err().code(),
            Some("router/invalid_request")
        );

        (h.handler)(declaration()).await.unwrap();
        let mut takeover = declaration();
        takeover["worker_id"] = json!("evil");
        assert_eq!(
            (h.handler)(takeover).await.unwrap_err().code(),
            Some("router/registration_rejected")
        );
    }

    #[tokio::test]
    async fn reregister_with_token_is_idempotent_and_preserves_operator_values() {
        let h = harness();
        let first = (h.handler)(declaration()).await.unwrap();
        let token = first["registration_token"].as_str().unwrap().to_string();
        h.bus
            .trigger("configuration::set", json!({ "id": "llm-router", "value": { "providers": { "anthropic": { "api_key": "sk" } } } }), None)
            .await
            .unwrap();
        let mut again = declaration();
        again["token"] = json!(token);
        let second = (h.handler)(again).await.unwrap();
        assert_eq!(second["registration_token"], json!(token));
        let entries = h.bus.config_entries.lock().unwrap();
        assert_eq!(
            entries.get("llm-router").unwrap().value,
            json!({ "providers": { "anthropic": { "api_key": "sk" } } })
        );
    }
}
