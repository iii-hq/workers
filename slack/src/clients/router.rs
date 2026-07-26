//! `router::models::list` — the sole source of available models/providers.

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Clone, Deserialize)]
pub struct Model {
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelsListResponse {
    #[serde(default)]
    models: Vec<Model>,
}

pub async fn list_models(iii: &IIIClient, timeout_ms: u64) -> Result<Vec<Model>, Error> {
    let request = TriggerRequest {
        function_id: "router::models::list".into(),
        payload: json!({ "capability": "tools" }),
        action: None,
        timeout_ms: Some(timeout_ms),
    };
    let value = match iii.namespace() {
        Some(ns) => iii.trigger(request.namespace(ns)).await?,
        None => iii.trigger(request).await?,
    };
    let resp: ModelsListResponse = serde_json::from_value(value)
        .map_err(|e| Error::Handler(format!("router::models::list: {e}")))?;
    Ok(resp.models)
}
