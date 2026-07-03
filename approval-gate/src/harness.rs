//! Thin wrappers around the harness's `harness::function::resolve` and
//! `harness::workspace::*` control-plane functions (contracts.md §
//! Workspace grant control-plane functions). All three workspace RPCs
//! respond `{session_id, roots: string[]}` — plain strings, no
//! timestamps/attribution.

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

pub async fn function_resolve(iii: &IIIClient, payload: Value) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: "harness::function::resolve".into(),
        payload,
        action: None,
        timeout_ms: None,
    })
    .await
}

fn parse_roots(reply: &Value) -> Vec<String> {
    reply
        .get("roots")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The session's current durable workspace grants (may be empty; never an
/// error on "no grants" — only on transport/RPC failure, e.g. an older
/// harness that doesn't implement this control plane yet).
pub async fn workspace_grants(iii: &IIIClient, session_id: &str) -> Result<Vec<String>, Error> {
    let reply = iii
        .trigger(TriggerRequest {
            function_id: "harness::workspace::grants".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(parse_roots(&reply))
}

/// Add a durable workspace grant for the session. Returns the updated root
/// list (unused by callers today, but kept for parity with the wire
/// contract and future use).
pub async fn workspace_grant(
    iii: &IIIClient,
    session_id: &str,
    root: &str,
) -> Result<Vec<String>, Error> {
    let reply = iii
        .trigger(TriggerRequest {
            function_id: "harness::workspace::grant".into(),
            payload: json!({ "session_id": session_id, "root": root }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(parse_roots(&reply))
}
