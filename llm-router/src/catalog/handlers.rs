//! The `router::models::list` / `get` / `supports` iii functions.
//!
//! Engine-backed coverage: tests/integration.rs (reconcile, restart restore).
use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::IIIError;
use serde_json::{json, Value};

use super::queries::{models_get, models_list, models_supports};
use super::store::CatalogStore;

pub fn make_models_list(
    catalog: Arc<CatalogStore>,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |raw: Value| {
        let catalog = catalog.clone();
        Box::pin(async move {
            let models = models_list(
                &catalog,
                raw.get("provider").and_then(Value::as_str),
                raw.get("capability").and_then(Value::as_str),
            )
            .await;
            Ok(json!({ "models": models }))
        })
    }
}

pub fn make_models_get(
    catalog: Arc<CatalogStore>,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |raw: Value| {
        let catalog = catalog.clone();
        Box::pin(async move {
            let model = models_get(
                &catalog,
                raw.get("provider").and_then(Value::as_str).unwrap_or(""),
                raw.get("id").and_then(Value::as_str).unwrap_or(""),
            )
            .await;
            Ok(match model {
                Some(m) => json!({ "model": m }),
                None => Value::Null, // null when unregistered (the cold-window signal)
            })
        })
    }
}

pub fn make_models_supports(
    catalog: Arc<CatalogStore>,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |raw: Value| {
        let catalog = catalog.clone();
        Box::pin(async move {
            let supported = models_supports(
                &catalog,
                raw.get("provider").and_then(Value::as_str).unwrap_or(""),
                raw.get("id").and_then(Value::as_str).unwrap_or(""),
                raw.get("capability").and_then(Value::as_str).unwrap_or(""),
            )
            .await;
            Ok(json!({ "supported": supported }))
        })
    }
}
