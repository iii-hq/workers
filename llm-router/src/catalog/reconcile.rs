//! The `router::models::reconcile` iii function — the only catalog write path.
//!
//! Engine-backed coverage: tests/integration.rs (reconcile + token gate).
use std::sync::Arc;

use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::{ModelsReconcileRequest, ModelsReconcileResponse};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use serde_json::json;

use crate::catalog::store::CatalogStore;
use crate::registry::store::RegistryStore;
use crate::triggers::{self, RouterEvents};

pub fn make_models_reconcile(
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
    events: Arc<RouterEvents>,
) -> impl Fn(ModelsReconcileRequest) -> BoxFuture<'static, Result<ModelsReconcileResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |req: ModelsReconcileRequest| {
        let (registry, catalog, events) = (registry.clone(), catalog.clone(), events.clone());
        Box::pin(async move {
            let provider = req.provider;
            registry
                .verify_token(&provider, req.token.as_deref())
                .await
                .map_err(Error::from)?;
            let models = req.models;
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
            events
                .emit(
                    triggers::MODELS_CHANGED,
                    json!({ "provider": provider, "count": count }),
                )
                .await;
            Ok(ModelsReconcileResponse { provider, count })
        })
    }
}
