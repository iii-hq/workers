//! Thin `state::get` / `state::set` / `state::delete` / `state::list`
//! wrappers around `iii.trigger()` (llm-router pattern). No explicit
//! timeout — every call uses the SDK default.

use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

pub async fn get(iii: &III, scope: &str, key: &str) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: None,
    })
    .await
}

pub async fn set(iii: &III, scope: &str, key: &str, value: Value) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms: None,
    })
    .await
}

pub async fn delete(iii: &III, scope: &str, key: &str) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: None,
    })
    .await
}

pub async fn list(iii: &III, scope: &str) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::list".into(),
        payload: json!({ "scope": scope }),
        action: None,
        timeout_ms: None,
    })
    .await
}
