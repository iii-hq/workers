use iii_sdk::CallRequest;
use iii_sdk::IIIClient;
use iii_sdk::errors::Error;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
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
    iii: &IIIClient,
    req: ResolveRequest,
    timeout_ms: u64,
) -> Result<ResolveResponse, Error> {
    let value = iii
        .call(CallRequest {
            function_id: "approval::resolve".into(),
            payload: serde_json::to_value(&req).unwrap_or(json!({})),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await?;
    serde_json::from_value(value).map_err(|e| Error::Handler(format!("approval::resolve: {e}")))
}

pub async fn approve_always(
    iii: &IIIClient,
    session_id: &str,
    function_id: &str,
    namespace: Option<&str>,
    timeout_ms: u64,
) -> Result<(), Error> {
    iii.call(CallRequest {
        function_id: "approval::approve-always".into(),
        payload: approve_always_payload(session_id, function_id, namespace),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await?;
    Ok(())
}

fn approve_always_payload(
    session_id: &str,
    function_id: &str,
    namespace: Option<&str>,
) -> serde_json::Value {
    let mut payload = json!({ "session_id": session_id, "function_id": function_id });
    if let Some(ns) = namespace {
        payload["namespace"] = json!(ns);
    }
    payload
}

pub async fn list_pending(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<Vec<PendingApprovalRecord>, Error> {
    let value = iii
        .call(CallRequest {
            function_id: "approval::list-pending".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await?;
    let resp: ListPendingResponse = serde_json::from_value(value)
        .map_err(|e| Error::Handler(format!("approval::list-pending: {e}")))?;
    Ok(resp.pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approve_always_carries_the_namespace_only_when_set() {
        assert_eq!(
            approve_always_payload("s", "f", Some("laptop"))["namespace"],
            "laptop"
        );
        assert!(
            approve_always_payload("s", "f", None)
                .get("namespace")
                .is_none()
        );
    }

    #[test]
    fn resolve_request_carries_the_namespace_only_when_set() {
        let req = |namespace: Option<&str>| ResolveRequest {
            session_id: "s".into(),
            function_call_id: "c".into(),
            decision: ResolveDecision::Allow,
            reason: None,
            namespace: namespace.map(str::to_string),
        };
        let with = serde_json::to_value(req(Some("laptop"))).unwrap();
        assert_eq!(with["namespace"], "laptop");
        let without = serde_json::to_value(req(None)).unwrap();
        assert!(without.get("namespace").is_none());
    }

    #[test]
    fn a_stored_callback_without_namespace_still_parses() {
        let data: crate::types::ApprovalCallbackData = serde_json::from_value(
            json!({ "session_id": "s", "function_call_id": "c", "function_id": "f" }),
        )
        .unwrap();
        assert_eq!(data.namespace, None);
        assert!(
            serde_json::to_value(&data)
                .unwrap()
                .get("namespace")
                .is_none()
        );
    }
}
