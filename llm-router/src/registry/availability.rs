//! Topology handler (bound to the engine's `subscribe` trigger on the
//! `engine::workers-available` topic) + the `router::provider::list` iii
//! function. Topology payload shapes vary (`worker_metadata_updated`,
//! connect/disconnect); we flip on any event naming a bound worker_id,
//! treating *disconnect* as down. Disconnect coverage is a verification risk
//! (design § risks) — dispatch-time flips in chat.rs are the fallback.
use std::sync::Arc;

use crate::types::router::{ProviderInfo, ProviderListResponse};
use serde_json::{json, Value};

use crate::bus::{handler, Bus, Handler};
use crate::registry::resolve::resolve_provider_config;
use crate::registry::store::RegistryStore;
use crate::triggers::TriggerEmitter;

pub fn make_on_worker_available(registry: Arc<RegistryStore>, emitter: TriggerEmitter) -> Handler {
    handler(move |raw: Value| {
        let (registry, emitter) = (registry.clone(), emitter.clone());
        async move {
            let Some(worker_id) = raw.get("worker_id").and_then(Value::as_str) else {
                return Ok(Value::Null); // unknown shapes are ignored
            };
            let providers = registry.providers_for_worker(worker_id).await;
            if providers.is_empty() {
                return Ok(Value::Null); // a worker with no registered provider creates nothing
            }
            let event = raw.get("event").and_then(Value::as_str).unwrap_or("");
            let available = !event.contains("disconnect");
            for id in providers {
                if registry.set_availability(&id, available).await {
                    emitter
                        .emit(
                            "router::provider::changed",
                            json!({ "provider": id, "op": if available { "available" } else { "unavailable" } }),
                        )
                        .await;
                }
            }
            Ok(Value::Null)
        }
    })
}

pub fn make_provider_list(bus: Arc<dyn Bus>, registry: Arc<RegistryStore>) -> Handler {
    handler(move |_raw: Value| {
        let (bus, registry) = (bus.clone(), registry.clone());
        async move {
            let mut providers = Vec::new();
            for rec in registry.list().await {
                let resolved = resolve_provider_config(&bus, &rec.declaration).await;
                providers.push(ProviderInfo {
                    id: rec.declaration.id.clone(),
                    display_name: rec
                        .declaration
                        .display_name
                        .clone()
                        .unwrap_or_else(|| rec.declaration.id.clone()),
                    configured: resolved.configured,
                    available: rec.available,
                    supports_model_listing: rec.declaration.supports_model_listing.unwrap_or(false),
                });
            }
            providers.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(serde_json::to_value(ProviderListResponse { providers })
                .expect("serializable response"))
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
    async fn topology_events_flip_only_the_bound_worker_and_list_reflects_state() {
        let bus = FakeBus::new();
        let registry = Arc::new(RegistryStore::new(bus.clone()));
        let catalog = Arc::new(CatalogStore::new(bus.clone()));
        let emitter = TriggerEmitter::install(bus.clone() as Arc<dyn crate::bus::Bus>, &*bus);
        let register = make_provider_register(
            bus.clone(),
            registry.clone(),
            catalog,
            emitter.clone(),
            EntryWriteLock::default(),
        );
        register(json!({ "id": "anthropic", "display_name": "Anthropic", "supports_model_listing": true, "worker_id": "w1", "credential_env_var": "UNSET_VAR_X" })).await.unwrap();

        let on_worker = make_on_worker_available(registry.clone(), emitter);
        let list = make_provider_list(bus, registry);

        // starts unavailable until its worker connects
        let res = list(json!({})).await.unwrap();
        assert_eq!(res["providers"][0]["available"], false);
        assert_eq!(res["providers"][0]["configured"], false);
        assert_eq!(res["providers"][0]["supports_model_listing"], true);

        on_worker(json!({ "event": "worker_connected", "worker_id": "w1" }))
            .await
            .unwrap();
        assert_eq!(
            list(json!({})).await.unwrap()["providers"][0]["available"],
            true
        );
        on_worker(json!({ "event": "worker_disconnected", "worker_id": "w1" }))
            .await
            .unwrap();
        assert_eq!(
            list(json!({})).await.unwrap()["providers"][0]["available"],
            false
        );
        // unrelated worker: no flip; malformed events: ignored
        on_worker(json!({ "event": "worker_connected", "worker_id": "other" }))
            .await
            .unwrap();
        assert_eq!(
            list(json!({})).await.unwrap()["providers"][0]["available"],
            false
        );
        on_worker(json!(null)).await.unwrap();
    }
}
