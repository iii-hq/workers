//! The `router::models::list` / `get` / `supports` iii functions.
//!
//! Engine-backed coverage: tests/integration.rs (reconcile, restart restore).
use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::errors::Error;

use crate::types::router::{
    ModelGetRequest, ModelGetResponse, ModelsListRequest, ModelsListResponse,
    ModelsSupportsRequest, ModelsSupportsResponse,
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
