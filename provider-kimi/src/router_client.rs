//! Thin wrappers over the router's provider-protocol functions. All calls
//! carry the registration token (identity binding, spec adaptation #1).
use crate::PROVIDER_ID;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::ProviderResolveResponse;
use serde_json::{json, Value};

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(15_000),
    })
    .await
}

/// `router::provider::resolve` — credential + effective settings.
pub async fn resolve(
    iii: &IIIClient,
    token: Option<&str>,
) -> Result<ProviderResolveResponse, Error> {
    let mut payload = json!({ "id": PROVIDER_ID });
    if let Some(t) = token {
        payload["token"] = json!(t);
    }
    let raw = call(iii, "router::provider::resolve", payload).await?;
    serde_json::from_value(raw).map_err(|e| Error::Remote {
        code: "provider/bad_resolve_response".into(),
        message: e.to_string(),
        stacktrace: None,
    })
}

/// `router::models::reconcile` — replace this provider's catalog slice.
pub async fn reconcile(
    iii: &IIIClient,
    models: Vec<Model>,
    token: Option<&str>,
) -> Result<(), Error> {
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
pub async fn models_get(iii: &IIIClient, model_id: &str) -> Option<Model> {
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
pub async fn register(iii: &IIIClient, declaration: Value) -> Result<Value, Error> {
    call(iii, "router::provider::register", declaration).await
}
