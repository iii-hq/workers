//! Delivery-tracking handlers.
//!
//! The six RPCs that orchestrators call to read and acknowledge the
//! gate's terminal-status records, plus the sweep that retires pending
//! ones when a session ends. They share two invariants:
//!
//! - Stamping `delivered_in_turn_id` is idempotent — re-acking a record
//!   that already has the stamp is a no-op.
//! - Lazy timeout flip: any read path through this module promotes
//!   pending-but-expired records to `timed_out` before applying its
//!   filter, so callers see expired calls surface on the same read they
//!   would have used regardless.

use serde_json::{json, Value};

use crate::lifecycle::{is_terminal_status, maybe_flip_timed_out, transition_record};
use crate::state::StateBus;
use crate::wire::pending_key;

/// List records currently in the `pending` status for a session. Used
/// by UIs to render the in-flight approval queue.
pub async fn handle_list_pending(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "pending": [] });
    }
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(state_scope, &prefix).await;
    let pending: Vec<Value> = all
        .into_iter()
        .filter(|v| v.get("status").and_then(Value::as_str) == Some("pending"))
        .collect();
    json!({ "pending": pending })
}

/// Default cap for `handle_list_undelivered` responses. A single LLM turn
/// should never be asked to ingest more than this many stitched approval
/// messages; older entries beyond the cap stay unacked and are reported via
/// the `omitted` counter so the caller can render a summary line.
pub const LIST_UNDELIVERED_DEFAULT_LIMIT: usize = 50;

/// Return terminal-status records for a session that haven't been stamped
/// with `delivered_in_turn_id`. Lazy timeout: pending records past
/// `expires_at` (as observed at `now_ms`) are flipped to `timed_out` before
/// the filter so they surface here in the same call.
///
/// Sorted oldest-first by `resolved_at` (records missing `resolved_at` sort
/// last as `u64::MAX`). Capped at `limit` (default
/// [`LIST_UNDELIVERED_DEFAULT_LIMIT`]); the response always includes an
/// `omitted` field counting entries left behind.
pub async fn handle_list_undelivered(
    bus: &dyn StateBus,
    state_scope: &str,
    payload: Value,
    now_ms: u64,
) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "entries": [], "omitted": 0 });
    }
    let limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(LIST_UNDELIVERED_DEFAULT_LIMIT);
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(state_scope, &prefix).await;
    let mut entries: Vec<Value> = Vec::new();
    for rec in all {
        // Defensive scope: some bus backends ignore the prefix and return
        // every record in `state_scope`. Drop anything not stamped with
        // the session_id we're listing for. Orphan records lacking a
        // session_id stamp are dropped (cannot be attributed); the
        // migration path that used to recover them no longer exists.
        match rec.get("session_id").and_then(Value::as_str) {
            Some(sid) if sid == session_id => {}
            _ => continue,
        }
        let rec = if let Some(flipped) = maybe_flip_timed_out(&rec, now_ms) {
            let call_id = flipped
                .get("function_call_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let _ = bus
                .set(
                    state_scope,
                    &pending_key(session_id, call_id),
                    flipped.clone(),
                )
                .await;
            flipped
        } else {
            rec
        };
        let status = rec.get("status").and_then(Value::as_str).unwrap_or("");
        if !is_terminal_status(status) {
            continue;
        }
        if rec
            .get("delivered_in_turn_id")
            .is_some_and(|v| !v.is_null())
        {
            continue;
        }
        entries.push(rec);
    }
    entries.sort_by_key(|e| {
        e.get("resolved_at")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX)
    });
    let total = entries.len();
    let omitted = total.saturating_sub(limit);
    entries.truncate(limit);
    json!({ "entries": entries, "omitted": omitted })
}

/// Stamp `delivered_in_turn_id` on terminal-status records named in
/// `call_ids` for the given session. Idempotent: records already stamped
/// (non-null `delivered_in_turn_id`) are not overwritten. Unknown call ids
/// are silently skipped.
pub async fn handle_ack_delivered(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let turn_id = payload.get("turn_id").and_then(Value::as_str).unwrap_or("");
    let call_ids: Vec<String> = payload
        .get("call_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if session_id.is_empty() || turn_id.is_empty() || call_ids.is_empty() {
        return json!({ "ok": true, "stamped": 0 });
    }
    let mut stamped = 0_u64;
    for cid in call_ids {
        let key = pending_key(session_id, &cid);
        let Some(rec) = bus.get(state_scope, &key).await else {
            continue;
        };
        if rec
            .get("delivered_in_turn_id")
            .is_some_and(|v| !v.is_null())
        {
            continue;
        }
        let mut next = rec;
        next.as_object_mut().unwrap().insert(
            "delivered_in_turn_id".into(),
            Value::String(turn_id.to_string()),
        );
        if bus.set(state_scope, &key, next).await.is_ok() {
            stamped += 1;
        }
    }
    json!({ "ok": true, "stamped": stamped })
}

/// Atomic list+ack: returns the same entries `handle_list_undelivered` would
/// surface (subject to the same FIFO+cap rules) and stamps each one with
/// `delivered_in_turn_id` before returning. Eliminates the list→LLM→ack
/// race window: if the caller crashes after receiving the response, the
/// entries are still considered delivered and will not resurface, which is
/// acceptable because terminal records are informational (the side-effect
/// already executed inside the gate).
///
/// Required payload: `{ session_id, turn_id, limit? }`.
pub async fn handle_consume_undelivered(
    bus: &dyn StateBus,
    state_scope: &str,
    payload: Value,
    now_ms: u64,
) -> Value {
    let turn_id = payload.get("turn_id").and_then(Value::as_str).unwrap_or("");
    if turn_id.is_empty() {
        return json!({ "ok": false, "error": "missing_turn_id", "entries": [], "omitted": 0 });
    }
    let listed = handle_list_undelivered(bus, state_scope, payload.clone(), now_ms).await;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let entries = listed["entries"].as_array().cloned().unwrap_or_default();
    let omitted = listed["omitted"].as_u64().unwrap_or(0);
    for rec in &entries {
        let cid = rec
            .get("function_call_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if cid.is_empty() {
            continue;
        }
        let key = pending_key(session_id, cid);
        let mut stamped = rec.clone();
        stamped.as_object_mut().unwrap().insert(
            "delivered_in_turn_id".into(),
            Value::String(turn_id.to_string()),
        );
        let _ = bus.set(state_scope, &key, stamped).await;
    }
    json!({ "ok": true, "entries": entries, "omitted": omitted })
}

/// One-shot drain: stamp every terminal-status record in `session_id` that
/// lacks `delivered_in_turn_id`. Intended for operator recovery after a
/// large backlog accumulates (e.g. when the orchestrator was offline or
/// `consume_undelivered` was unreachable). Pending records are untouched —
/// use `sweep_session` if you want to expire them first.
pub async fn handle_flush_delivered(
    bus: &dyn StateBus,
    state_scope: &str,
    payload: Value,
) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let turn_id = payload.get("turn_id").and_then(Value::as_str).unwrap_or("");
    if session_id.is_empty() || turn_id.is_empty() {
        return json!({ "ok": false, "error": "missing_session_or_turn_id", "stamped": 0 });
    }
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(state_scope, &prefix).await;
    let mut stamped = 0_u64;
    for rec in all {
        let status = rec.get("status").and_then(Value::as_str).unwrap_or("");
        if !is_terminal_status(status) {
            continue;
        }
        if rec
            .get("delivered_in_turn_id")
            .is_some_and(|v| !v.is_null())
        {
            continue;
        }
        let cid = rec
            .get("function_call_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default();
        if cid.is_empty() {
            continue;
        }
        let mut next = rec;
        next.as_object_mut().unwrap().insert(
            "delivered_in_turn_id".into(),
            Value::String(turn_id.to_string()),
        );
        if bus
            .set(state_scope, &pending_key(session_id, &cid), next)
            .await
            .is_ok()
        {
            stamped += 1;
        }
    }
    json!({ "ok": true, "stamped": stamped })
}

/// Sweep all still-pending approvals for a session to timed_out.
///
/// The `timed_out` status is self-describing per the Denial refactor —
/// callers no longer pass (or get back) a reason string. If you need to
/// distinguish *why* a session was swept (delete vs. abort vs. timeout),
/// the calling worker already has that context and should log it there.
pub async fn handle_sweep_session(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "ok": false, "error": "missing_session_id", "swept": 0 });
    }
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(state_scope, &prefix).await;
    let mut swept = 0_u64;
    for rec in all {
        if rec.get("status").and_then(Value::as_str) != Some("pending") {
            continue;
        }
        let call_id = rec
            .get("function_call_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if call_id.is_empty() {
            continue;
        }
        let flipped = transition_record(&rec, "timed_out", None, None, None);
        if bus
            .set(state_scope, &pending_key(session_id, call_id), flipped)
            .await
            .is_ok()
        {
            swept += 1;
        }
    }
    json!({ "ok": true, "swept": swept })
}
