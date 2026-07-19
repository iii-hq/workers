use serde_json::{json, Map, Value};

use crate::types::recorder::{RecorderEventKind, RecorderEventV1};

use super::helpers::literal_subset;
use super::{Evidence, GradeOutcome};

pub(super) fn grade_generations_consumed(
    params: &Map<String, Value>,
    evidence: &Evidence,
) -> GradeOutcome {
    let want = params.get("count").and_then(Value::as_u64);
    let expected = json!({
        "count": want,
        "unused_expectations": 0,
    });
    let actual = json!({
        "count": evidence.generations_consumed,
        "unused_expectations": evidence.generations_total - evidence.generations_consumed,
    });
    let passed = Some(evidence.generations_consumed) == want
        && evidence.generations_consumed == evidence.generations_total;
    (passed, expected, actual, vec!["router-calls.json"])
}

pub(super) fn grade_target_calls(params: &Map<String, Value>, evidence: &Evidence) -> GradeOutcome {
    let function_id = params
        .get("function_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let want_count = params.get("count").and_then(Value::as_u64).unwrap_or(0);
    let calls: Vec<&RecorderEventV1> = evidence
        .recorder_events
        .iter()
        .filter(|event| {
            event.kind == RecorderEventKind::TargetCall && event.function_id == function_id
        })
        .collect();
    let payload_ok = match params.get("payload") {
        Some(want) => calls.iter().all(|call| &call.payload == want),
        None => true,
    };
    let subset_ok = match params.get("payload_subset") {
        Some(want) => calls.iter().all(|call| literal_subset(want, &call.payload)),
        None => true,
    };
    let expected = Value::Object(params.clone());
    let actual = json!({
        "count": calls.len(),
        "payloads": calls.iter().map(|call| call.payload.clone()).collect::<Vec<_>>(),
    });
    let passed = calls.len() as u64 == want_count && payload_ok && subset_ok;
    (passed, expected, actual, vec!["target-calls.json"])
}
