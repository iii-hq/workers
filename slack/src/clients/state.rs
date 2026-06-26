//! Thin wrapper over the engine `state::*` functions (durable KV).

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

pub async fn get(iii: &IIIClient, scope: &str, key: &str, timeout_ms: u64) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await
}

pub async fn set(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    value: Value,
    timeout_ms: u64,
) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await?;
    Ok(())
}

pub async fn delete(iii: &IIIClient, scope: &str, key: &str, timeout_ms: u64) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await?;
    Ok(())
}

pub async fn get_string(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    timeout_ms: u64,
) -> Option<String> {
    match get(iii, scope, key, timeout_ms).await {
        Ok(v) => v.as_str().map(str::to_string),
        Err(_) => None,
    }
}

pub async fn get_value(iii: &IIIClient, scope: &str, key: &str, timeout_ms: u64) -> Option<Value> {
    match get(iii, scope, key, timeout_ms).await {
        Ok(v) if !v.is_null() => Some(v),
        _ => None,
    }
}
