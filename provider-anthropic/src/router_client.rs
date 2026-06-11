//! Thin wrappers over the router's provider-protocol functions. All calls
//! carry the registration token (identity binding, spec adaptation #1).
use crate::PROVIDER_ID;
use iii_sdk::{IIIError, TriggerRequest, III};
use llm_router::types::model::Model;
use llm_router::types::router::ProviderResolveResponse;
use serde_json::{json, Value};

async fn call(iii: &III, function_id: &str, payload: Value) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(15_000),
    })
    .await
}

/// `router::provider::resolve` — credential + effective settings.
pub async fn resolve(iii: &III, token: Option<&str>) -> Result<ProviderResolveResponse, IIIError> {
    let mut payload = json!({ "id": PROVIDER_ID });
    if let Some(t) = token {
        payload["token"] = json!(t);
    }
    let raw = call(iii, "router::provider::resolve", payload).await?;
    serde_json::from_value(raw).map_err(|e| IIIError::Remote {
        code: "provider/bad_resolve_response".into(),
        message: e.to_string(),
        stacktrace: None,
    })
}

/// `router::models::reconcile` — replace this provider's catalog slice.
pub async fn reconcile(iii: &III, models: Vec<Model>, token: Option<&str>) -> Result<(), IIIError> {
    let mut payload = json!({
        "provider": PROVIDER_ID,
        "models": serde_json::to_value(models).expect("serializable models"),
    });
    if let Some(t) = token {
        payload["token"] = json!(t);
    }
    call(iii, "router::models::reconcile", payload).await?;
    Ok(())
}

/// `router::models::get` — authoritative catalog record (None when absent).
pub async fn models_get(iii: &III, model_id: &str) -> Option<Model> {
    let raw = call(
        iii,
        "router::models::get",
        json!({ "provider": PROVIDER_ID, "id": model_id }),
    )
    .await
    .ok()?;
    serde_json::from_value(raw.get("model")?.clone()).ok()
}

/// `router::provider::register` — returns the registration token to persist.
pub async fn register(iii: &III, declaration: Value) -> Result<Value, IIIError> {
    call(iii, "router::provider::register", declaration).await
}
