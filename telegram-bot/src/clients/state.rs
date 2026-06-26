use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

pub async fn get(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    timeout_ms: Option<u64>,
) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms,
    })
    .await
}

pub async fn set(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    value: Value,
    timeout_ms: Option<u64>,
) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms,
    })
    .await
}

pub async fn delete(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    timeout_ms: Option<u64>,
) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms,
    })
    .await
}

pub async fn get_string(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    timeout_ms: u64,
) -> Option<String> {
    match get(iii, scope, key, Some(timeout_ms)).await {
        Ok(v) if v.is_string() => v.as_str().map(str::to_string),
        _ => None,
    }
}

pub async fn get_i64(iii: &IIIClient, scope: &str, key: &str, timeout_ms: u64) -> Option<i64> {
    match get(iii, scope, key, Some(timeout_ms)).await {
        Ok(v) if v.is_i64() => v.as_i64(),
        Ok(v) if v.is_u64() => v.as_u64().and_then(|n| i64::try_from(n).ok()),
        _ => None,
    }
}
