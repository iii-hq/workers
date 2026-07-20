use std::collections::BTreeSet;

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
    let expected_status = params
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    let expected = json!({
        "delivery_count_valid": true,
        "all_deliveries_identical_after_timestamp_normalization": true,
        "contract_shape_valid": true,
        "function_status_session_and_turn_match": true,
        "sequence_order_valid": true,
    });
    let lifecycle: Vec<_> = evidence
        .recorder_events
        .iter()
        .filter(|event| event.kind == RecorderEventKind::Lifecycle)
        .collect();
    let payloads: Vec<Value> = lifecycle
        .iter()
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
    let bound_ok = lifecycle.iter().all(|event| {
        event.function_id == "integration-recorder::lifecycle"
            && event.payload.get("status").and_then(Value::as_str) == Some(expected_status)
            && event.payload.get("session_id").and_then(Value::as_str)
                == Some(evidence.session_id.as_str())
            && match &evidence.turn_id {
                Some(turn) => event.payload.get("turn_id").and_then(Value::as_str) == Some(turn),
                None => false,
            }
    });
    let allowed_keys = BTreeSet::from([
        "session_id",
        "turn_id",
        "status",
        "timestamp",
        "result",
        "result_error",
        "reason",
        "parent",
        "parent_session_id",
        "reactive_depth",
    ]);
    let contract_shape_valid = payloads.iter().all(|payload| {
        let Some(map) = payload.as_object() else {
            return false;
        };
        map.get("timestamp").and_then(Value::as_i64).is_some()
            && map.keys().all(|key| allowed_keys.contains(key.as_str()))
    });
    let sequence_monotonic = lifecycle
        .windows(2)
        .all(|window| window[0].sequence < window[1].sequence);
    let last_target_sequence = evidence
        .recorder_events
        .iter()
        .filter(|event| event.kind == RecorderEventKind::TargetCall)
        .map(|event| event.sequence)
        .max()
        .unwrap_or(0);
    let follows_target_calls = lifecycle
        .first()
        .is_none_or(|event| event.sequence > last_target_sequence);
    let sequence_order_valid = sequence_monotonic && follows_target_calls;
    let delivery_count_valid = if allow_identical_duplicates {
        !payloads.is_empty()
    } else {
        payloads.len() == 1
    };
    let actual = json!({
        "delivery_count_valid": delivery_count_valid,
        "all_deliveries_identical_after_timestamp_normalization": identical,
        "contract_shape_valid": contract_shape_valid,
        "function_status_session_and_turn_match": bound_ok,
        "sequence_order_valid": sequence_order_valid,
    });
    let passed = delivery_count_valid
        && identical
        && contract_shape_valid
        && bound_ok
        && sequence_order_valid;
    (passed, expected, actual, vec!["lifecycle-events.json"])
}
