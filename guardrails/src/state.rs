use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::Value;

#[allow(dead_code)]
pub async fn state_get(iii: &III, scope: &str, key: &str) -> Result<Value, IIIError> {
    let payload = serde_json::json!({
        "scope": scope,
        "key": key,
    });
    iii.trigger(TriggerRequest {
        function_id: "state::get".to_string(),
        payload,
        action: None,
        timeout_ms: Some(5000),
    })
    .await
}

pub async fn state_set(
    iii: &III,
    scope: &str,
    key: &str,
    value: Value,
) -> Result<Value, IIIError> {
    let payload = serde_json::json!({
        "scope": scope,
        "key": key,
        "value": value,
    });
    iii.trigger(TriggerRequest {
        function_id: "state::set".to_string(),
        payload,
        action: None,
        timeout_ms: Some(5000),
    })
    .await
}
