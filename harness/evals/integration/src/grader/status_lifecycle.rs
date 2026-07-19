use serde_json::{json, Map, Value};

use crate::types::recorder::RecorderEventKind;

use super::{Evidence, GradeOutcome};

pub(super) fn grade_terminal(params: &Map<String, Value>, evidence: &Evidence) -> GradeOutcome {
    let expected = Value::Object(params.clone());
    let actual = json!({
        "status": evidence.status.get("status").cloned().unwrap_or(Value::Null),
        "pending_calls": evidence
            .status
            .get("pending_function_calls")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
        "queued_messages": evidence
            .status
            .get("queued")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
        "children": evidence
            .status
            .get("children")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
    });
    let passed = actual.get("status") == params.get("status")
        && actual.get("pending_calls") == params.get("pending_calls")
        && actual.get("queued_messages") == params.get("queued_messages")
        && actual.get("children").and_then(Value::as_u64).unwrap_or(0) == 0;
    (passed, expected, actual, vec!["status.json"])
}

pub(super) fn grade_completed_once(
    params: &Map<String, Value>,
    evidence: &Evidence,
) -> GradeOutcome {
    let allow_identical_duplicates = params
        .get("allow_identical_duplicates")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let expected = json!({
        "delivery_count_valid": true,
        "all_deliveries_identical_after_timestamp_normalization": true,
        "session_and_turn_match": true,
    });
    let payloads: Vec<Value> = evidence
        .recorder_events
        .iter()
        .filter(|event| event.kind == RecorderEventKind::Lifecycle)
        .map(|event| event.payload.clone())
        .collect();
    let normalized: Vec<Value> = payloads
        .iter()
        .map(|payload| {
            let mut copy = payload.clone();
            if let Some(map) = copy.as_object_mut() {
                map.remove("timestamp");
            }
            copy
        })
        .collect();
    let identical = normalized.windows(2).all(|window| window[0] == window[1]);
    let bound_ok = payloads.iter().all(|payload| {
        payload.get("session_id").and_then(Value::as_str) == Some(evidence.session_id.as_str())
            && match &evidence.turn_id {
                Some(turn) => payload.get("turn_id").and_then(Value::as_str) == Some(turn),
                None => false,
            }
    });
    let delivery_count_valid = if allow_identical_duplicates {
        !payloads.is_empty()
    } else {
        payloads.len() == 1
    };
    let actual = json!({
        "delivery_count_valid": delivery_count_valid,
        "all_deliveries_identical_after_timestamp_normalization": identical,
        "session_and_turn_match": bound_ok,
    });
    let passed = delivery_count_valid && identical && bound_ok;
    (passed, expected, actual, vec!["lifecycle-events.json"])
}
