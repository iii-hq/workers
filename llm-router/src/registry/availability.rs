//! `router::provider::list` — registered providers with configured/available status.
use std::sync::Arc;

use crate::types::router::{ProviderInfo, ProviderListRequest, ProviderListResponse};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;

use crate::config::state::{snapshot, ConfigCell};
use crate::registry::resolve::resolve_provider_config;
use crate::registry::store::RegistryStore;

pub fn make_provider_list(
    config: ConfigCell,
    registry: Arc<RegistryStore>,
) -> impl Fn(ProviderListRequest) -> BoxFuture<'static, Result<ProviderListResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req: ProviderListRequest| {
        let (config, registry) = (config.clone(), registry.clone());
        Box::pin(async move {
            let config = snapshot(&config);
            let mut providers = Vec::new();
            for rec in registry.list().await {
                let resolved = resolve_provider_config(&config, &rec.declaration);
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
