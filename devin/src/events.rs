//! Stream emitter: writes frames via the engine's `stream::set` builtin with a
//! per-process epoch + a process-wide monotonic sequence so item_ids never
//! collide across restarts. Failures are logged, not propagated — the streams
//! are best-effort observability; the function's return value and the session
//! record are the source of truth.
//!
//! `stream::set` is provided by the `iii-stream` worker, which an engine may
//! not run. When it is not registered the emitter logs that once and skips the
//! call (re-probing periodically) instead of failing and warning per frame.

use std::sync::atomic::{AtomicU64, Ordering};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::optional::OptionalDependency;
use crate::wire::now_ms;

static EPOCH: Lazy<String> = Lazy::new(|| Uuid::new_v4().to_string());
// Process-wide counter: globally unique without retaining per-session state.
static SEQ: AtomicU64 = AtomicU64::new(0);
static STREAM: OptionalDependency = OptionalDependency::new("stream::set");

fn next_item_id(session_id: &str) -> String {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{session_id}-{}-{seq:08}", *EPOCH)
}

pub async fn emit(iii: &IIIClient, stream_name: &str, session_id: &str, data: Value) {
    if !STREAM.should_try(now_ms()) {
        return;
    }
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
    let error = res.err();
    if STREAM.observe(error.as_ref(), now_ms()) {
        return;
    }
    if let Some(e) = error {
        tracing::warn!(stream_name, session_id, error = %e, "stream::set failed");
    }
}
