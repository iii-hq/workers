//! Thin `state::get` / `state::set` wrappers around `iii.trigger()`
//! (binary-worker.md § 7). All router state lives under the "llm-router"
//! scope; the router is the only writer of its keys (single-instance worker).
use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

pub const STATE_SCOPE: &str = "llm-router";

pub async fn state_get(iii: &III, key: &str) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".into(),
        payload: json!({ "scope": STATE_SCOPE, "key": key }),
        action: None,
        timeout_ms: None,
    })
    .await
}

pub async fn state_set(iii: &III, key: &str, value: Value) -> Result<(), IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": STATE_SCOPE, "key": key, "value": value }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}
