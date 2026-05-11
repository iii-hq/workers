use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

pub async fn subscribe(iii: Arc<III>, payload: Value) -> Result<Value, IIIError> {
    let since_ms = payload
        .get("since_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let snapshot = super::call(&iii, "engine::workers::list", json!({}))
        .await
        .map_err(|e| IIIError::Handler(format!("engine::workers::list failed: {e}")))?;

    let workers = snapshot
        .get("workers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let events: Vec<Value> = workers
        .into_iter()
        .map(|w| {
            json!({
                "kind": "snapshot",
                "worker": w.get("name").cloned().unwrap_or(Value::Null),
                "status": w.get("status").cloned().unwrap_or(Value::Null),
                "function_count": w.get("function_count").cloned().unwrap_or(json!(0)),
            })
        })
        .collect();

    Ok(json!({
        "since_ms": since_ms,
        "channel": "introspection.registrations",
        "note": "Snapshot only. Live stream wires through engine pubsub channel introspection.registrations once engine emits registration events on that topic.",
        "events": events,
    }))
}
