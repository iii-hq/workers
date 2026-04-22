use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::Value;

pub async fn state_get(iii: &III, key: &str) -> Result<Value, IIIError> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "state::get".to_string(),
            payload: serde_json::json!({
                "scope": "introspect",
                "key": key
            }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(result)
}

pub async fn state_set(iii: &III, key: &str, value: Value) -> Result<Value, IIIError> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "state::set".to_string(),
            payload: serde_json::json!({
                "scope": "introspect",
                "key": key,
                "value": value
            }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(result)
}
