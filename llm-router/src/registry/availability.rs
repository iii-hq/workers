//! `router::provider::list` — registered providers with configured/available status.
use std::sync::Arc;

use crate::types::router::{ProviderInfo, ProviderListRequest, ProviderListResponse};
use futures::future::BoxFuture;
use iii_sdk::{IIIError, III};

use crate::registry::resolve::resolve_provider_config;
use crate::registry::store::RegistryStore;

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
