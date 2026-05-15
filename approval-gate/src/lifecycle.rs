//! Persisted-record lifecycle helpers.
//!
//! Pure functions that construct and transition the `Value`-blob record
//! schema as it lives in the iii state bus. No I/O, no async — the only
//! impurity is reading the system clock via [`transition_record`], whose
//! testable variant [`transition_record_with_now`] takes `now_ms`
//! directly. (Operators adopting the typed schema can read the same
//! shape via [`crate::record::Record`] / [`crate::record::Record::from_value`].)
//!
//! The wire keys (`status`, `function_call_id`, `expires_at`,
//! `resolved_at`, `result`, `error`, `denial`, `delivered_in_turn_id`)
//! are stable contract; renaming requires a state-store migration. The
//! `denial` field is documented at [`crate::wire::Denial`].

use serde_json::{json, Value};

use crate::wire::{pending_key, Denial};

/// True if `status` is one of the terminal states a stitched system message
/// should be built from. `pending` and `approved` are intermediate.
pub fn is_terminal_status(status: &str) -> bool {
    matches!(status, "executed" | "failed" | "denied" | "timed_out")
}

/// Build a fresh pending record. `session_id` is unset here —
/// `handle_intercept` stamps it before persisting. `expires_at` is
/// `now_ms + timeout_ms`, saturating on overflow so a buggy caller
/// can't underflow the deadline.
pub fn build_pending_record(
    function_call_id: &str,
    function_id: &str,
    args: &Value,
    now_ms: u64,
    timeout_ms: u64,
) -> Value {
    json!({
        "function_call_id": function_call_id,
        "function_id": function_id,
        "args": args,
        "status": "pending",
        "expires_at": now_ms.saturating_add(timeout_ms),
    })
}

/// Build a new record by transitioning a pending base record to a terminal
/// status. All terminal fields (`result`, `error`, `denial`) are optional;
/// only the ones provided are attached. Existing fields on the base
/// (including `delivered_in_turn_id` and `resolved_at` if present) are
/// preserved. The first transition into a terminal status stamps
/// `resolved_at`.
pub fn transition_record(
    base: &Value,
    new_status: &str,
    result: Option<Value>,
    error: Option<String>,
    denial: Option<Denial>,
) -> Value {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    transition_record_with_now(base, new_status, result, error, denial, now_ms)
}

/// Testable variant of [`transition_record`] that takes `now_ms` directly.
pub fn transition_record_with_now(
    base: &Value,
    new_status: &str,
    result: Option<Value>,
    error: Option<String>,
    denial: Option<Denial>,
    now_ms: u64,
) -> Value {
    let mut rec = base.clone();
    if let Some(obj) = rec.as_object_mut() {
        obj.insert("status".into(), Value::String(new_status.to_string()));
        if let Some(r) = result {
            obj.insert("result".into(), r);
        }
        if let Some(e) = error {
            obj.insert("error".into(), Value::String(e));
        }
        if let Some(d) = denial {
            obj.insert(
                "denial".into(),
                serde_json::to_value(&d).expect("Denial is always serializable"),
            );
        }
        if is_terminal_status(new_status) && !obj.contains_key("resolved_at") {
            obj.insert("resolved_at".into(), Value::Number(now_ms.into()));
        }
    }
    rec
}

/// For a bag of pending records, return the subset that have expired at
/// `now_ms` along with the metadata needed to commit the flip and notify the
/// owning session. Records without a stamped `session_id` (legacy rows
/// written before that field existed) are skipped — they'll still be picked
/// up lazily by `handle_list_undelivered` on the next read.
pub fn collect_timed_out_for_sweep(
    records: &[Value],
    now_ms: u64,
) -> Vec<(String, Value, String, String)> {
    records
        .iter()
        .filter_map(|rec| {
            let flipped = maybe_flip_timed_out(rec, now_ms)?;
            let session_id = flipped
                .get("session_id")
                .and_then(Value::as_str)?
                .to_string();
            let function_call_id = flipped
                .get("function_call_id")
                .and_then(Value::as_str)?
                .to_string();
            if session_id.is_empty() || function_call_id.is_empty() {
                return None;
            }
            let key = pending_key(&session_id, &function_call_id);
            Some((key, flipped, session_id, function_call_id))
        })
        .collect()
}

/// Return Some(timed_out_record) if `rec` is pending and `now_ms` is past
/// `expires_at`; otherwise None. Pure function — does not write state.
pub fn maybe_flip_timed_out(rec: &Value, now_ms: u64) -> Option<Value> {
    if rec.get("status").and_then(Value::as_str) != Some("pending") {
        return None;
    }
    let exp = rec.get("expires_at").and_then(Value::as_u64)?;
    if now_ms < exp {
        return None;
    }
    // Timeout flip carries no Denial: the `timed_out` status itself is the
    // explanation. Downstream renderers (turn-orchestrator stitching, UIs)
    // branch on the status, not on a redundant reason string.
    Some(transition_record(rec, "timed_out", None, None, None))
}
