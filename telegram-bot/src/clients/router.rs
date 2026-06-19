use iii_sdk::{IIIError, TriggerRequest, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ModelsListRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Model {
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub supports_thinking: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ModelsListResponse {
    pub models: Vec<Model>,
}

pub async fn list_models(iii: &III, timeout_ms: u64) -> Result<Vec<Model>, IIIError> {
    let value = iii
        .trigger(TriggerRequest {
            function_id: "router::models::list".into(),
            payload: json!({ "capability": "tools" }),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await?;
    let resp: ModelsListResponse = serde_json::from_value(value)
        .map_err(|e| IIIError::Handler(format!("router::models::list: {e}")))?;
    Ok(resp.models)
}
