//! `session-inbox::push`, `session-inbox::drain`, `session-inbox::peek` handlers.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::json;

use crate::config::WorkerConfig;
use crate::{inbox_key, DRAIN_ID, PEEK_ID, PUSH_ID};

async fn execute_push(iii: III, state_scope: String, payload: Value) -> Result<Value, IIIError> {
    let name = required_str(&payload, "name")?;
    let session_id = required_str(&payload, "session_id")?;
    let item = payload
        .get("item")
        .cloned()
        .ok_or_else(|| IIIError::Handler("missing required field: item".into()))?;
    let key = inbox_key(&name, &session_id);

    iii.trigger(TriggerRequest {
        function_id: "state::update".into(),
        payload: json!({
            "scope": state_scope,
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

async fn execute_peek(iii: III, state_scope: String, payload: Value) -> Result<Value, IIIError> {
    let name = required_str(&payload, "name")?;
    let session_id = required_str(&payload, "session_id")?;
    let key = inbox_key(&name, &session_id);
    let value = match iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": state_scope, "key": key }),
            action: None,
            timeout_ms: None,
        })
        .await
    {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(error = %e, %name, %session_id, "inbox: state::get failed; treating as empty");
            None
        }
    };
    let items = value.filter(|v| !v.is_null()).unwrap_or_else(|| json!([]));
    Ok(json!({ "items": items }))
}

/// Atomically swap the inbox to `[]` and return whatever was there.
///
/// Implemented via `state::update` with a single `Set { path: "", value: [] }`
/// op. The engine returns `old_value` (whatever was there) and `new_value`
/// (`[]`). One round-trip, no race window between read and reset.
async fn execute_drain(iii: III, state_scope: String, payload: Value) -> Result<Value, IIIError> {
    let name = required_str(&payload, "name")?;
    let session_id = required_str(&payload, "session_id")?;
    let key = inbox_key(&name, &session_id);
    let resp = match iii
        .trigger(TriggerRequest {
            function_id: "state::update".into(),
            payload: json!({
                "scope": state_scope,
                "key": key,
                "ops": [{ "type": "set", "path": "", "value": [] }],
            }),
            action: None,
            timeout_ms: None,
        })
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, %name, %session_id, "session-inbox::drain: state::update failed; treating as empty");
            return Ok(json!({ "items": [] }));
        }
    };
    let items = resp
        .get("old_value")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json!([]));
    Ok(json!({ "items": items }))
}

pub fn register(iii: &III, cfg: &Arc<WorkerConfig>) {
    let iii_push = iii.clone();
    let scope_push = cfg.state_scope.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(PUSH_ID.to_string())
            .with_description("Append an item to a session-scoped inbox.".to_string()),
        move |payload: Value| {
            let iii = iii_push.clone();
            let state_scope = scope_push.clone();
            async move { execute_push(iii, state_scope, payload).await }
        },
    ));
    let iii_peek = iii.clone();
    let scope_peek = cfg.state_scope.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(PEEK_ID.to_string()).with_description(
            "Read all items in a session-scoped inbox without mutating.".to_string(),
        ),
        move |payload: Value| {
            let iii = iii_peek.clone();
            let state_scope = scope_peek.clone();
            async move { execute_peek(iii, state_scope, payload).await }
        },
    ));
    let iii_drain = iii.clone();
    let scope_drain = cfg.state_scope.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(DRAIN_ID.to_string()).with_description(
            "Atomically read and clear all items in a session-scoped inbox.".to_string(),
        ),
        move |payload: Value| {
            let iii = iii_drain.clone();
            let state_scope = scope_drain.clone();
            async move { execute_drain(iii, state_scope, payload).await }
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
