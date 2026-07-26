//! `approval::*` — resolve held tool calls (optional `approval-gate` worker).

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::json;

pub async fn resolve(
    iii: &IIIClient,
    session_id: &str,
    function_call_id: &str,
    allow: bool,
    timeout_ms: u64,
) -> Result<(), Error> {
    let request = TriggerRequest {
        function_id: "approval::resolve".into(),
        payload: json!({
            "session_id": session_id,
            "function_call_id": function_call_id,
            "decision": if allow { "allow" } else { "deny" },
        }),
        action: None,
        timeout_ms: Some(timeout_ms),
    };
    match iii.namespace() {
        Some(ns) => iii.trigger(request.namespace(ns)).await?,
        None => iii.trigger(request).await?,
    };
    Ok(())
}

pub async fn approve_always(
    iii: &IIIClient,
    session_id: &str,
    function_id: &str,
    timeout_ms: u64,
) -> Result<(), Error> {
    let request = TriggerRequest {
        function_id: "approval::approve-always".into(),
        payload: json!({ "session_id": session_id, "function_id": function_id }),
        action: None,
        timeout_ms: Some(timeout_ms),
    };
    match iii.namespace() {
        Some(ns) => iii.trigger(request.namespace(ns)).await?,
        None => iii.trigger(request).await?,
    };
    Ok(())
}
