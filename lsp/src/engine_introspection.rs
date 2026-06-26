use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Function information returned by `engine::functions::list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub function_id: String,
    pub description: Option<String>,
    pub request_format: Option<Value>,
    pub response_format: Option<Value>,
    pub metadata: Option<Value>,
}

/// Trigger instance information returned by `engine::registered-triggers::list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerInfo {
    pub id: String,
    pub trigger_type: String,
    pub function_id: String,
    pub config: Value,
    pub metadata: Option<Value>,
}

/// Trigger type information returned by `engine::triggers::list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerTypeInfo {
    pub id: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_request_format: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_request_format: Option<Value>,
}

/// Worker information returned by `engine::workers::list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub id: String,
    pub name: Option<String>,
    pub runtime: Option<String>,
    pub version: Option<String>,
    pub os: Option<String>,
    pub ip_address: Option<String>,
    pub status: String,
    pub connected_at_ms: u64,
    pub function_count: usize,
    pub functions: Vec<String>,
    pub active_invocations: usize,
    #[serde(default)]
    pub isolation: Option<String>,
}

pub async fn list_functions(iii: &IIIClient) -> Result<Vec<FunctionInfo>, Error> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".into(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(result
        .get("functions")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

pub async fn list_workers(iii: &IIIClient) -> Result<Vec<WorkerInfo>, Error> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "engine::workers::list".into(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(result
        .get("workers")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

pub async fn list_triggers(
    iii: &IIIClient,
    include_internal: bool,
) -> Result<Vec<TriggerInfo>, Error> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "engine::registered-triggers::list".into(),
            payload: serde_json::json!({ "include_internal": include_internal }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(result
        .get("registered_triggers")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

pub async fn list_trigger_types(
    iii: &IIIClient,
    include_internal: bool,
) -> Result<Vec<TriggerTypeInfo>, Error> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "engine::triggers::list".into(),
            payload: serde_json::json!({ "include_internal": include_internal }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(result
        .get("triggers")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}
