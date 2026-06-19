use iii_sdk::{IIIError, TriggerRequest, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::types::{PendingApprovalRecord, ResolveDecision};

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ResolveRequest {
    pub session_id: String,
    pub function_call_id: String,
    pub decision: ResolveDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ResolveResponse {
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ApproveAlwaysRequest {
    pub session_id: String,
    pub function_id: String,
}

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct ListPendingRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ListPendingResponse {
    pub pending: Vec<PendingApprovalRecord>,
}

pub async fn resolve(
    iii: &III,
    req: ResolveRequest,
    timeout_ms: u64,
) -> Result<ResolveResponse, IIIError> {
    let value = iii
        .trigger(TriggerRequest {
            function_id: "approval::resolve".into(),
            payload: serde_json::to_value(&req).unwrap_or(json!({})),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await?;
    serde_json::from_value(value).map_err(|e| IIIError::Handler(format!("approval::resolve: {e}")))
}

pub async fn approve_always(
    iii: &III,
    session_id: &str,
    function_id: &str,
    timeout_ms: u64,
) -> Result<(), IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "approval::approve-always".into(),
        payload: json!({
            "session_id": session_id,
            "function_id": function_id,
        }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await?;
    Ok(())
}

pub async fn list_pending(
    iii: &III,
    session_id: &str,
    timeout_ms: u64,
) -> Result<Vec<PendingApprovalRecord>, IIIError> {
    let value = iii
        .trigger(TriggerRequest {
            function_id: "approval::list-pending".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await?;
    let resp: ListPendingResponse = serde_json::from_value(value)
        .map_err(|e| IIIError::Handler(format!("approval::list-pending: {e}")))?;
    Ok(resp.pending)
}
