//! `queue::push`, `queue::drain`, `queue::peek` handlers.

use iii_sdk::{IIIError, RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::json;

use crate::{queue_key, DRAIN_ID, PEEK_ID, PUSH_ID};

const STATE_SCOPE: &str = "agent";

async fn execute_push(iii: III, payload: Value) -> Result<Value, IIIError> {
    let name = required_str(&payload, "name")?;
    let session_id = required_str(&payload, "session_id")?;
    let item = payload
        .get("item")
        .cloned()
        .ok_or_else(|| IIIError::Handler("missing required field: item".into()))?;
    let key = queue_key(&name, &session_id);

    iii.trigger(TriggerRequest {
        function_id: "state::update".into(),
        payload: json!({
            "scope": STATE_SCOPE,
            "key": key,
            "ops": [{ "type": "append", "path": "", "value": item }],
        }),
        action: None,
        timeout_ms: None,
    })
    .await
    .map_err(|e| IIIError::Handler(e.to_string()))?;
    Ok(json!({ "ok": true }))
}

async fn execute_peek(iii: III, payload: Value) -> Result<Value, IIIError> {
    let name = required_str(&payload, "name")?;
    let session_id = required_str(&payload, "session_id")?;
    let key = queue_key(&name, &session_id);
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
            tracing::warn!(error = %e, %name, %session_id, "queue: state::get failed; treating as empty");
            None
        }
    };
    let items = value.filter(|v| !v.is_null()).unwrap_or_else(|| json!([]));
    Ok(json!({ "items": items }))
}

async fn execute_drain(iii: III, payload: Value) -> Result<Value, IIIError> {
    let name = required_str(&payload, "name")?;
    let session_id = required_str(&payload, "session_id")?;
    let key = queue_key(&name, &session_id);
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
            tracing::warn!(error = %e, %name, %session_id, "queue: state::get failed; treating as empty");
            None
        }
    };
    let items = value.filter(|v| !v.is_null()).unwrap_or_else(|| json!([]));
    if items.as_array().map_or(false, |a| !a.is_empty()) {
        if let Err(e) = iii
            .trigger(TriggerRequest {
                function_id: "state::set".into(),
                payload: json!({ "scope": STATE_SCOPE, "key": key, "value": [] }),
                action: None,
                timeout_ms: None,
            })
            .await
        {
            tracing::warn!(error = %e, %name, %session_id, "queue::drain: state::set reset failed; items may redeliver");
        }
    }
    Ok(json!({ "items": items }))
}

pub fn register(iii: &III) {
    let iii_push = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(PUSH_ID.to_string())
            .with_description("Append an item to a session-scoped durable queue.".to_string()),
        move |payload: Value| {
            let iii = iii_push.clone();
            async move { execute_push(iii, payload).await }
        },
    ));
    let iii_peek = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(PEEK_ID.to_string()).with_description(
            "Read all items in a session-scoped queue without mutating.".to_string(),
        ),
        move |payload: Value| {
            let iii = iii_peek.clone();
            async move { execute_peek(iii, payload).await }
        },
    ));
    let iii_drain = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(DRAIN_ID.to_string())
            .with_description("Read and clear all items in a session-scoped queue.".to_string()),
        move |payload: Value| {
            let iii = iii_drain.clone();
            async move { execute_drain(iii, payload).await }
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
        let v = json!({ "name": "steering" });
        assert_eq!(required_str(&v, "name").unwrap(), "steering");
    }

    #[test]
    fn required_str_errors_on_missing_field() {
        let v = json!({});
        let err = required_str(&v, "name").unwrap_err();
        assert!(err.to_string().contains("name"));
    }
}
