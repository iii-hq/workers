//! Thin wrappers around the harness's `harness::function::resolve` and
//! `harness::filesystem::*` control-plane functions (contracts.md §
//! Filesystem grant control-plane functions). All three filesystem RPCs
//! respond `{session_id, roots: string[]}` — plain strings, no
//! timestamps/attribution.

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

pub async fn function_resolve(iii: &IIIClient, payload: Value) -> Result<Value, Error> {
    let request = TriggerRequest {
        function_id: "harness::function::resolve".into(),
        payload,
        action: None,
        timeout_ms: None,
    };
    match iii.namespace() {
        Some(ns) => iii.trigger(request.namespace(ns)).await,
        None => iii.trigger(request).await,
    }
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

/// The session's current durable filesystem grants (may be empty; never an
/// error on "no grants" — only on transport/RPC failure, e.g. an older
/// harness that doesn't implement this control plane yet).
pub async fn filesystem_grants(iii: &IIIClient, session_id: &str) -> Result<Vec<String>, Error> {
    let request = TriggerRequest {
        function_id: "harness::filesystem::grants".into(),
        payload: json!({ "session_id": session_id }),
        action: None,
        timeout_ms: None,
    };
    let reply = match iii.namespace() {
        Some(ns) => iii.trigger(request.namespace(ns)).await?,
        None => iii.trigger(request).await?,
    };
    Ok(parse_roots(&reply))
}

/// Add a durable filesystem grant for the session. Returns the updated root
/// list (unused by callers today, but kept for parity with the wire
/// contract and future use).
pub async fn filesystem_grant(
    iii: &IIIClient,
    session_id: &str,
    root: &str,
) -> Result<Vec<String>, Error> {
    let request = TriggerRequest {
        function_id: "harness::filesystem::grant".into(),
        payload: json!({ "session_id": session_id, "root": root }),
        action: None,
        timeout_ms: None,
    };
    let reply = match iii.namespace() {
        Some(ns) => iii.trigger(request.namespace(ns)).await?,
        None => iii.trigger(request).await?,
    };
    Ok(parse_roots(&reply))
}
