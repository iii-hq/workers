//! Stream emitter: writes frames via the engine's `stream::set` builtin with a
//! per-process epoch + per-session monotonic sequence so item_ids never
//! collide across restarts (matches the harness emitter). Failures are logged,
//! not propagated — the streams are best-effort observability; the function's
//! return value and the session record are the source of truth.

use std::collections::HashMap;
use std::sync::Mutex;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use uuid::Uuid;

static EPOCH: Lazy<String> = Lazy::new(|| Uuid::new_v4().to_string());
static SEQ: Lazy<Mutex<HashMap<String, u64>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn next_item_id(session_id: &str) -> String {
    let mut map = SEQ.lock().expect("seq mutex");
    let n = map.entry(session_id.to_string()).or_insert(0);
    let seq = *n;
    *n += 1;
    format!("{session_id}-{}-{seq:08}", *EPOCH)
}

pub async fn emit(iii: &IIIClient, stream_name: &str, session_id: &str, data: Value) {
    let item_id = next_item_id(session_id);
    let res = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".to_string(),
            payload: json!({
                "stream_name": stream_name,
                "group_id": session_id,
                "item_id": item_id,
                "data": data,
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    if let Err(e) = res {
        tracing::warn!(stream_name, session_id, error = %e, "stream::set failed");
    }
}
