//! The `router::models::reconcile` iii function — the only catalog write path.
//!
//! Engine-backed coverage: tests/integration.rs (reconcile + token gate).
use std::sync::Arc;

use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::{ModelsReconcileRequest, ModelsReconcileResponse};
use futures::future::BoxFuture;
use iii_sdk::{IIIError, III};
use serde_json::json;

use crate::catalog::store::CatalogStore;
use crate::registry::store::RegistryStore;
use crate::triggers;

pub fn make_models_reconcile(
    iii: III,
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
) -> impl Fn(ModelsReconcileRequest) -> BoxFuture<'static, Result<ModelsReconcileResponse, IIIError>>
       + Send
       + Sync
       + 'static {
    move |req: ModelsReconcileRequest| {
        let (iii, registry, catalog) = (iii.clone(), registry.clone(), catalog.clone());
        Box::pin(async move {
            let provider = req.provider;
            registry
                .verify_token(&provider, req.token.as_deref())
                .await
                .map_err(IIIError::from)?;
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
            triggers::publish(
                &iii,
                triggers::MODELS_CHANGED,
                json!({ "provider": provider, "count": count }),
            )
            .await;
            Ok(ModelsReconcileResponse { provider, count })
        })
    }
}
