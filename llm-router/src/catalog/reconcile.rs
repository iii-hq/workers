//! The `router::models::reconcile` iii function — the only catalog write path.
use std::sync::Arc;

use crate::types::errors::{RouterCode, RouterError};
use crate::types::model::Model;
use serde_json::{json, Value};

use crate::bus::{handler, BusError, Handler};
use crate::catalog::store::CatalogStore;
use crate::registry::store::RegistryStore;
use crate::triggers::TriggerEmitter;

pub fn make_models_reconcile(
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
    emitter: TriggerEmitter,
) -> Handler {
    handler(move |raw: Value| {
        let (registry, catalog, emitter) = (registry.clone(), catalog.clone(), emitter.clone());
        async move {
            let provider = raw
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let token = raw.get("token").and_then(Value::as_str).map(String::from);
            registry
                .verify_token(&provider, token.as_deref())
                .await
                .map_err(BusError::from)?;
            let models: Vec<Model> = serde_json::from_value(
                raw.get("models").cloned().unwrap_or(Value::Null),
            )
            .map_err(|e| {
                BusError::from(RouterError::new(
                    RouterCode::InvalidRequest,
                    format!("models must be an array of Model: {e}"),
                ))
            })?;
            for m in &models {
                if m.provider != provider {
                    return Err(RouterError::new(
                        RouterCode::InvalidRequest,
                        format!(
                            "model {} declares provider {}, expected {provider}",
                            m.id, m.provider
                        ),
                    )
                    .into());
                }
            }
            let count = models.len();
            catalog.set_slice(&provider, models).await?;
            emitter
                .emit(
                    "router::models::changed",
                    json!({ "provider": provider, "count": count }),
                )
                .await;
            Ok(json!({ "provider": provider, "count": count }))
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

    #[tokio::test]
    async fn replaces_slice_in_one_write_gates_token_and_validates_provider_field() {
        let bus = FakeBus::new();
        let registry = Arc::new(RegistryStore::new(bus.clone()));
        let catalog = Arc::new(CatalogStore::new(bus.clone()));
        let emitter = TriggerEmitter::install(bus.clone() as Arc<dyn crate::bus::Bus>, &*bus);
        let register = make_provider_register(
            bus.clone(),
            registry.clone(),
            catalog.clone(),
            emitter.clone(),
            EntryWriteLock::default(),
        );
        let token = register(json!({ "id": "anthropic" })).await.unwrap()["registration_token"]
            .as_str()
            .unwrap()
            .to_string();
        let reconcile = make_models_reconcile(registry, catalog.clone(), emitter);

        let model = json!({ "id": "a", "provider": "anthropic", "context_window": 1, "max_output_tokens": 1 });
        let res = reconcile(json!({ "provider": "anthropic", "token": token, "models": [model] }))
            .await
            .unwrap();
        assert_eq!(res, json!({ "provider": "anthropic", "count": 1 }));
        assert_eq!(catalog.slice("anthropic").await.len(), 1);

        // reconcile-to-empty evicts (the auth-failure path)
        reconcile(json!({ "provider": "anthropic", "token": token, "models": [] }))
            .await
            .unwrap();
        assert!(catalog.slice("anthropic").await.is_empty());

        let err = reconcile(json!({ "provider": "anthropic", "token": "no", "models": [] }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Some("router/registration_rejected"));
        let wrong =
            json!({ "id": "x", "provider": "other", "context_window": 1, "max_output_tokens": 1 });
        let err = reconcile(json!({ "provider": "anthropic", "token": token, "models": [wrong] }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Some("router/invalid_request"));
    }
}
