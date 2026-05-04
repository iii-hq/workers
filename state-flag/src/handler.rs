//! `flag::set`, `flag::clear`, `flag::is_set` handlers.

use iii_sdk::{IIIError, RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::json;

use crate::{flag_key, is_set as decode_is_set, CLEAR_ID, IS_SET_ID, SET_ID};

const STATE_SCOPE: &str = "agent";

async fn execute_set(iii: III, payload: Value, value: bool) -> Result<Value, IIIError> {
    let name = required_str(&payload, "name")?;
    let session_id = required_str(&payload, "session_id")?;
    let key = flag_key(&name, &session_id);
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": STATE_SCOPE, "key": key, "value": value }),
        action: None,
        timeout_ms: None,
    })
    .await
    .map_err(|e| IIIError::Handler(e.to_string()))?;
    Ok(json!({ "ok": true }))
}

async fn execute_is_set(iii: III, payload: Value) -> Result<Value, IIIError> {
    let name = required_str(&payload, "name")?;
    let session_id = required_str(&payload, "session_id")?;
    let key = flag_key(&name, &session_id);
    let value = match iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": STATE_SCOPE, "key": key }),
            action: None,
            timeout_ms: None,
        })
        .await
    {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(error = %e, %name, %session_id, "flag::is_set: state::get failed; treating as unset");
            None
        }
    };
    Ok(json!({ "value": decode_is_set(value.as_ref()) }))
}

pub fn register(iii: &III) {
    let iii_set = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(SET_ID.to_string())
            .with_description("Set a session-scoped boolean flag to true.".to_string()),
        move |payload: Value| {
            let iii = iii_set.clone();
            async move { execute_set(iii, payload, true).await }
        },
    ));
    let iii_clear = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(CLEAR_ID.to_string())
            .with_description("Clear a session-scoped boolean flag (set to false).".to_string()),
        move |payload: Value| {
            let iii = iii_clear.clone();
            async move { execute_set(iii, payload, false).await }
        },
    ));
    let iii_is_set = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(IS_SET_ID.to_string())
            .with_description("Read a session-scoped boolean flag.".to_string()),
        move |payload: Value| {
            let iii = iii_is_set.clone();
            async move { execute_is_set(iii, payload).await }
        },
    ));
}

fn required_str(payload: &Value, field: &str) -> Result<String, IIIError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| IIIError::Handler(format!("missing required field: {field}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_str_extracts_present_field() {
        let v = json!({ "name": "abort" });
        assert_eq!(required_str(&v, "name").unwrap(), "abort");
    }

    #[test]
    fn required_str_errors_on_missing_field() {
        let v = json!({});
        let err = required_str(&v, "name").unwrap_err();
        assert!(err.to_string().contains("name"));
    }
}
