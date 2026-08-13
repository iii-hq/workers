//! `router::provider::list` — registered providers with configured/available status.
use std::sync::Arc;

use crate::types::router::{ProviderInfo, ProviderListRequest, ProviderListResponse};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;

use crate::catalog::store::CatalogStore;
use crate::config::state::{snapshot, ConfigCell};
use crate::registry::resolve::resolve_provider_config;
use crate::registry::store::RegistryStore;
use crate::types::router::{CatalogState, CredentialRequirement, CredentialState};

pub fn make_provider_list(
    config: ConfigCell,
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
) -> impl Fn(ProviderListRequest) -> BoxFuture<'static, Result<ProviderListResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req: ProviderListRequest| {
        let (config, registry, catalog) = (config.clone(), registry.clone(), catalog.clone());
        Box::pin(async move {
            let config = snapshot(&config);
            let mut providers = Vec::new();
            for rec in registry.list().await {
                let resolved = resolve_provider_config(&config, &rec.declaration);
                let models = catalog.slice(&rec.declaration.id).await;
                let mut diagnostic = rec.diagnostic.clone();
                diagnostic.credential_state = match rec.declaration.credential_requirement {
                    CredentialRequirement::External => CredentialState::External,
                    CredentialRequirement::Optional => CredentialState::Ready,
                    CredentialRequirement::Required if resolved.configured => {
                        CredentialState::Ready
                    }
                    CredentialRequirement::Required => CredentialState::Missing,
                };
                if diagnostic.catalog_state == CatalogState::Unknown && !models.is_empty() {
                    diagnostic.catalog_state = CatalogState::Ready;
                }
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
                    credential_requirement: rec.declaration.credential_requirement,
                    model_count: models.len(),
                    diagnostic,
                });
            }
            providers.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(ProviderListResponse { providers })
        })
    }
}
