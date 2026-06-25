//! Thin `state::get` / `state::set` / `state::delete` / `state::list`
//! wrappers around `iii.trigger()` (llm-router pattern). No explicit
//! timeout — every call uses the SDK default.

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

pub async fn get(iii: &IIIClient, scope: &str, key: &str) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: None,
    })
    .await
}

pub async fn set(iii: &IIIClient, scope: &str, key: &str, value: Value) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms: None,
    })
    .await
}

pub async fn delete(iii: &IIIClient, scope: &str, key: &str) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: None,
    })
    .await
}

pub async fn list(iii: &IIIClient, scope: &str) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::list".into(),
        payload: json!({ "scope": scope }),
        action: None,
        timeout_ms: None,
    })
    .await
}
