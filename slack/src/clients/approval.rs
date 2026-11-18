//! `approval::*` — resolve held tool calls (optional `approval-gate` worker).

use iii_sdk::CallRequest;
use iii_sdk::IIIClient;
use iii_sdk::errors::Error;
use serde_json::json;

pub async fn resolve(
    iii: &IIIClient,
    session_id: &str,
    function_call_id: &str,
    allow: bool,
    namespace: Option<&str>,
    timeout_ms: u64,
) -> Result<(), Error> {
    let mut payload = json!({
        "session_id": session_id,
        "function_call_id": function_call_id,
        "decision": if allow { "allow" } else { "deny" },
    });
    if let Some(ns) = namespace {
        payload["namespace"] = json!(ns);
    }
    iii.call(CallRequest {
        function_id: "approval::resolve".into(),
        payload,
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await?;
    Ok(())
}

pub async fn approve_always(
    iii: &IIIClient,
    session_id: &str,
    function_id: &str,
    namespace: Option<&str>,
    timeout_ms: u64,
) -> Result<(), Error> {
    let mut payload = json!({ "session_id": session_id, "function_id": function_id });
    if let Some(ns) = namespace {
        payload["namespace"] = json!(ns);
    }
    iii.call(CallRequest {
        function_id: "approval::approve-always".into(),
        payload,
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await?;
    Ok(())
}
