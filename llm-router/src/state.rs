//! Thin `state::get` / `state::set` wrappers around `iii.call()`
//! (binary-worker.md § 7). All router state lives under the "llm-router"
//! scope; the router is the only writer of its keys (single-instance worker).
use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, IIIClient};
use serde_json::{json, Value};

pub const STATE_SCOPE: &str = "llm-router";

pub async fn state_get(iii: &IIIClient, key: &str) -> Result<Value, Error> {
    iii.call(CallRequest {
        function_id: "state::get".into(),
        payload: json!({ "scope": STATE_SCOPE, "key": key }),
        action: None,
        timeout_ms: None,
    })
    .await
}

pub async fn state_set(iii: &IIIClient, key: &str, value: Value) -> Result<(), Error> {
    iii.call(CallRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": STATE_SCOPE, "key": key, "value": value }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}
