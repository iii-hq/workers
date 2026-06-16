//! Topology handler (bound to the engine's `subscribe` trigger on the
//! `engine::workers-available` topic) + the `router::provider::list` iii
//! function. Topology payload shapes vary (`worker_metadata_updated`,
//! connect/disconnect); we flip on any event naming a bound worker_id,
//! treating *disconnect* as down. Disconnect coverage is a verification risk
//! (design § risks) — dispatch-time flips in chat.rs are the fallback.
use std::sync::Arc;

use crate::types::router::{
    ProviderInfo, ProviderListRequest, ProviderListResponse, RouterAck, WorkerAvailableEvent,
};
use futures::future::BoxFuture;
use iii_sdk::{IIIError, III};
use serde_json::json;

use crate::registry::resolve::resolve_provider_config;
use crate::registry::store::RegistryStore;
use crate::triggers;

pub fn make_on_worker_available(
    iii: III,
    registry: Arc<RegistryStore>,
) -> impl Fn(WorkerAvailableEvent) -> BoxFuture<'static, Result<RouterAck, IIIError>>
       + Send
       + Sync
       + 'static {
    move |event: WorkerAvailableEvent| {
        let (iii, registry) = (iii.clone(), registry.clone());
        Box::pin(async move {
            let Some(worker_id) = event.worker_id.as_deref() else {
                return Ok(RouterAck { ok: true }); // unknown shapes are ignored
            };
            let providers = registry.providers_for_worker(worker_id).await;
            if providers.is_empty() {
                return Ok(RouterAck { ok: true }); // a worker with no registered provider creates nothing
            }
            let event_name = event.event.as_deref().unwrap_or("");
            let available = !event_name.contains("disconnect");
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
            Ok(RouterAck { ok: true })
        })
    }
}

pub fn make_provider_list(
    iii: III,
    registry: Arc<RegistryStore>,
) -> impl Fn(ProviderListRequest) -> BoxFuture<'static, Result<ProviderListResponse, IIIError>>
       + Send
       + Sync
       + 'static {
    move |_req: ProviderListRequest| {
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
            Ok(ProviderListResponse { providers })
        })
    }
}
