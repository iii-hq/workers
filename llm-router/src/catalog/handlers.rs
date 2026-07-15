//! The `router::models::list` / `get` / `supports` iii functions.
//!
//! Engine-backed coverage: tests/integration.rs (reconcile, restart restore).
use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::errors::Error;

use crate::types::router::{
    ModelBudgetRequest, ModelBudgetResponse, ModelGetRequest, ModelGetResponse, ModelsListRequest,
    ModelsListResponse, ModelsSupportsRequest, ModelsSupportsResponse,
};
use crate::{
    chat::output_tokens::resolve_max_output_tokens,
    config::state::{snapshot, ConfigCell},
    registry::store::RegistryStore,
};

use super::queries::{models_get, models_list, models_supports};
use super::store::CatalogStore;

pub fn make_models_list(
    catalog: Arc<CatalogStore>,
) -> impl Fn(ModelsListRequest) -> BoxFuture<'static, Result<ModelsListResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |req: ModelsListRequest| {
        let catalog = catalog.clone();
        Box::pin(async move {
            let models =
                models_list(&catalog, req.provider.as_deref(), req.capability.as_deref()).await;
            Ok(ModelsListResponse { models })
        })
    }
}

pub fn make_models_get(
    catalog: Arc<CatalogStore>,
) -> impl Fn(ModelGetRequest) -> BoxFuture<'static, Result<Option<ModelGetResponse>, Error>>
       + Send
       + Sync
       + 'static {
    move |req: ModelGetRequest| {
        let catalog = catalog.clone();
        Box::pin(async move {
            // provider defaults to "" on the wire — treat empty as unset so
            // id-only lookups resolve when the id is unambiguous.
            let provider = (!req.provider.is_empty()).then_some(req.provider.as_str());
            let model = models_get(&catalog, provider, &req.id).await;
            // null when unregistered (the cold-window signal)
            Ok(model.map(|model| ModelGetResponse { model }))
        })
    }
}

pub fn make_models_budget(
    catalog: Arc<CatalogStore>,
    registry: Arc<RegistryStore>,
    config: ConfigCell,
) -> impl Fn(ModelBudgetRequest) -> BoxFuture<'static, Result<Option<ModelBudgetResponse>, Error>>
       + Send
       + Sync
       + 'static {
    move |req: ModelBudgetRequest| {
        let (catalog, registry, config) = (catalog.clone(), registry.clone(), config.clone());
        Box::pin(async move {
            let provider = (!req.provider.is_empty()).then_some(req.provider.as_str());
            let Some(model) = models_get(&catalog, provider, &req.id).await else {
                return Ok(None);
            };
            let Some(record) = registry.get(&model.provider).await else {
                return Ok(None);
            };
            let config = snapshot(&config);
            let effective_max_output_tokens = resolve_max_output_tokens(
                req.max_output_tokens,
                config
                    .provider_slice(&model.provider)
                    .and_then(|slice| slice.get("max_tokens"))
                    .and_then(serde_json::Value::as_u64),
                Some(model.max_output_tokens),
                record
                    .declaration
                    .defaults
                    .as_ref()
                    .and_then(|defaults| defaults.max_tokens)
                    .unwrap_or(8192),
                config.settings().output_token_max,
            );
            Ok(Some(ModelBudgetResponse {
                model,
                effective_max_output_tokens,
            }))
        })
    }
}

pub fn make_models_supports(
    catalog: Arc<CatalogStore>,
) -> impl Fn(ModelsSupportsRequest) -> BoxFuture<'static, Result<ModelsSupportsResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |req: ModelsSupportsRequest| {
        let catalog = catalog.clone();
        Box::pin(async move {
            let supported =
                models_supports(&catalog, &req.provider, &req.id, &req.capability).await;
            Ok(ModelsSupportsResponse { supported })
        })
    }
}
