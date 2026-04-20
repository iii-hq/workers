use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{Value, json};

pub async fn state_get(iii: &III, scope: &str, key: &str) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".to_string(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(5000),
    })
    .await
}

pub async fn state_set(iii: &III, scope: &str, key: &str, value: &Value) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".to_string(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms: Some(5000),
    })
    .await
}

pub async fn state_delete(iii: &III, scope: &str, key: &str) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".to_string(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(5000),
    })
    .await
}

pub async fn state_list(iii: &III, scope: &str) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::list".to_string(),
        payload: json!({ "scope": scope }),
        action: None,
        timeout_ms: Some(5000),
    })
    .await
}
