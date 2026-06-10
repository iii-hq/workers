//! The `router::models::reconcile` iii function — the only catalog write path.
//!
//! Engine-backed coverage: tests/integration.rs (reconcile + token gate).
use std::sync::Arc;

use crate::types::errors::{RouterCode, RouterError};
use crate::types::model::Model;
use futures::future::BoxFuture;
use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use crate::catalog::store::CatalogStore;
use crate::registry::store::RegistryStore;
use crate::triggers;

pub fn make_models_reconcile(
    iii: III,
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |raw: Value| {
        let (iii, registry, catalog) = (iii.clone(), registry.clone(), catalog.clone());
        Box::pin(async move {
            let provider = raw
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let token = raw.get("token").and_then(Value::as_str).map(String::from);
            registry
                .verify_token(&provider, token.as_deref())
                .await
                .map_err(IIIError::from)?;
            let models: Vec<Model> = serde_json::from_value(
                raw.get("models").cloned().unwrap_or(Value::Null),
            )
            .map_err(|e| {
                IIIError::from(RouterError::new(
                    RouterCode::InvalidRequest,
                    format!("models must be an array of Model: {e}"),
                ))
            })?;
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
            Ok(json!({ "provider": provider, "count": count }))
        })
    }
}
