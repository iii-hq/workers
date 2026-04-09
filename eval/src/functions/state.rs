use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

pub async fn state_get(iii: &III, scope: &str, key: &str) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".to_string(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(5000),
    })
    .await
}

pub async fn state_set(iii: &III, scope: &str, key: &str, value: Value) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".to_string(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms: Some(5000),
    })
    .await
}
