//! Topology handler (bound to the engine's `subscribe` trigger on the
//! `engine::workers-available` topic) + the `router::provider::list` iii
//! function. Topology payload shapes vary (`worker_metadata_updated`,
//! connect/disconnect); we flip on any event naming a bound worker_id,
//! treating *disconnect* as down. Disconnect coverage is a verification risk
//! (design § risks) — dispatch-time flips in chat.rs are the fallback.
use std::sync::Arc;

use crate::types::router::{ProviderInfo, ProviderListResponse};
use futures::future::BoxFuture;
use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use crate::registry::resolve::resolve_provider_config;
use crate::registry::store::RegistryStore;
use crate::triggers;

pub fn make_on_worker_available(
    iii: III,
    registry: Arc<RegistryStore>,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |raw: Value| {
        let (iii, registry) = (iii.clone(), registry.clone());
        Box::pin(async move {
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
                    triggers::publish(
                        &iii,
                        triggers::PROVIDER_CHANGED,
                        json!({ "provider": id, "op": if available { "available" } else { "unavailable" } }),
                    )
                    .await;
                }
            }
            Ok(Value::Null)
        })
    }
}

pub fn make_provider_list(
    iii: III,
    registry: Arc<RegistryStore>,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |_raw: Value| {
        let (iii, registry) = (iii.clone(), registry.clone());
        Box::pin(async move {
            let mut providers = Vec::new();
            for rec in registry.list().await {
                let resolved = resolve_provider_config(&iii, &rec.declaration).await;
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
        })
    }
}
