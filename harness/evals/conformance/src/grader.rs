//! Pure-code grading (spec § Oracles): every invariant reads collected
//! evidence only — no engine calls, no subject mutation. The invariant id
//! registry here is the concrete vocabulary for `scenario.yaml`'s
//! `invariants` array (the spec left it open; recorded in the amendment).

use serde_json::{json, Map, Value};

use crate::types::recorder::{RecorderEventKind, RecorderEventV1};
use crate::types::scenario::{InvariantResultV1, InvariantSpecV1};

pub const KNOWN_INVARIANTS: [&str; 10] = [
    "send.flags",
    "transcript.message_counts",
    "transcript.assistant_text",
    "transcript.no_duplicates",
    "transcript.function_result",
    "transcript.calls_closed",
    "status.terminal",
    "lifecycle.completed_once",
    "router.generations_consumed",
    "target.calls",
];

/// Everything the grader may look at, exactly as persisted to the artifact
/// directory (grading twice over the same evidence is byte-deterministic).
pub struct Evidence {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub send_response: Option<Value>,
    /// Final `harness::status` report (JSON null when the session is unknown).
    pub status: Value,
    /// All transcript `MessageItem`s across pages, in order.
    pub transcript: Vec<Value>,
    pub generations_consumed: u64,
    pub generations_total: u64,
    pub recorder_events: Vec<RecorderEventV1>,
}

pub fn grade(specs: &[InvariantSpecV1], evidence: &Evidence) -> Vec<InvariantResultV1> {
    specs
        .iter()
        .map(|spec| {
            let (passed, expected, actual, refs) = grade_one(spec, evidence);
            InvariantResultV1 {
                id: spec.id.clone(),
                passed,
                expected,
                actual,
                evidence_refs: refs.into_iter().map(String::from).collect(),
            }
        })
        .collect()
}

fn grade_one(
    spec: &InvariantSpecV1,
    evidence: &Evidence,
) -> (bool, Value, Value, Vec<&'static str>) {
    let params = &spec.parameters;
    match spec.id.as_str() {
        "send.flags" => {
            let expected = json!({
                "accepted": true,
                "merged": flag(params, "merged"),
                "queued": flag(params, "queued"),
                "deduplicated": flag(params, "deduplicated"),
            });
            // Absent optional flags normalize to false (spec § send contract).
            let actual = match &evidence.send_response {
                Some(resp) => json!({
                    "accepted": resp.get("accepted") == Some(&Value::Bool(true)),
                    "merged": resp.get("merged") == Some(&Value::Bool(true)),
                    "queued": resp.get("queued") == Some(&Value::Bool(true)),
                    "deduplicated": resp.get("deduplicated") == Some(&Value::Bool(true)),
                }),
                None => Value::Null,
            };
            (
                actual == expected,
                expected,
                actual,
                vec!["send-response.json"],
            )
        }

        "transcript.message_counts" => {
            let mut counts = std::collections::BTreeMap::new();
            for role in ["user", "assistant", "function_result"] {
                counts.insert(role.to_string(), 0u64);
            }
            for item in &evidence.transcript {
                if let Some(role) = item
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(Value::as_str)
                {
                    *counts.entry(role.to_string()).or_insert(0) += 1;
                }
            }
            let expected = Value::Object(params.clone());
            let actual = serde_json::to_value(&counts).expect("counts serialize");
            let passed = params.iter().all(|(role, want)| {
                counts.get(role).copied().unwrap_or(0) == want.as_u64().unwrap_or(u64::MAX)
            });
            (passed, expected, actual, vec!["transcript.json"])
        }

        "transcript.assistant_text" => {
            let expected = Value::Object(params.clone());
            let last_assistant = evidence
                .transcript
                .iter()
                .filter_map(|item| item.get("message"))
                .rfind(|m| m.get("role").and_then(Value::as_str) == Some("assistant"));
            let actual = match last_assistant {
                Some(message) => {
                    let text: String = message
                        .get("content")
                        .and_then(Value::as_array)
                        .map(|blocks| {
                            blocks
                                .iter()
                                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                                .filter_map(|b| b.get("text").and_then(Value::as_str))
                                .collect()
                        })
                        .unwrap_or_default();
                    json!({ "text": text, "usage": message.get("usage").cloned().unwrap_or(Value::Null) })
                }
                None => Value::Null,
            };
            let passed = match &actual {
                Value::Null => false,
                actual => {
                    actual.get("text") == params.get("text")
                        && params.get("usage").is_none_or(|want| {
                            subset_eq(want, actual.get("usage").unwrap_or(&Value::Null))
                        })
                }
            };
            (passed, expected, actual, vec!["transcript.json"])
        }

        "transcript.no_duplicates" => {
            let mut seen = std::collections::BTreeSet::new();
            let mut duplicates = Vec::new();
            for item in &evidence.transcript {
                if let Some(entry_id) = item.get("entry_id").and_then(Value::as_str) {
                    if !seen.insert(entry_id.to_string()) {
                        duplicates.push(entry_id.to_string());
                    }
                }
            }
            let passed = duplicates.is_empty();
            (
                passed,
                json!({ "duplicate_entry_ids": [] }),
                json!({ "duplicate_entry_ids": duplicates }),
                vec!["transcript.json"],
            )
        }

        // Every dispatched `function_call` must be closed by a durable
        // `function_result` referencing its id — an interrupted call left
        // dangling poisons every later provider request on the session
        // (iii-hq/workers#507).
        "transcript.calls_closed" => {
            let mut call_ids = Vec::new();
            let mut result_ids = std::collections::BTreeSet::new();
            for item in &evidence.transcript {
                let Some(message) = item.get("message") else {
                    continue;
                };
                match message.get("role").and_then(Value::as_str) {
                    Some("assistant") => {
                        for block in message
                            .get("content")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            if block.get("type").and_then(Value::as_str) == Some("function_call") {
                                if let Some(id) = block.get("id").and_then(Value::as_str) {
                                    call_ids.push(id.to_string());
                                }
                            }
                        }
                    }
                    Some("function_result") => {
                        if let Some(id) = message.get("function_call_id").and_then(Value::as_str) {
                            result_ids.insert(id.to_string());
                        }
                    }
                    _ => {}
                }
            }
            let dangling: Vec<&String> = call_ids
                .iter()
                .filter(|id| !result_ids.contains(*id))
                .collect();
            let passed = dangling.is_empty();
            (
                passed,
                json!({ "dangling_function_calls": [] }),
                json!({
                    "dangling_function_calls": dangling,
                    "calls": call_ids,
                    "results": result_ids,
                }),
                vec!["transcript.json"],
            )
        }

        "transcript.function_result" => {
            let expected = Value::Object(params.clone());
            let results: Vec<&Value> = evidence
                .transcript
                .iter()
                .filter_map(|item| item.get("message"))
                .filter(|m| m.get("role").and_then(Value::as_str) == Some("function_result"))
                .collect();
            let actual = json!(results);
            let passed = results.len() == 1 && subset_eq(&expected, results[0]);
            (passed, expected, actual, vec!["transcript.json"])
        }

        "status.terminal" => {
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

        "lifecycle.completed_once" => {
            let expected = json!({
                "at_least_one_completion": true,
                "all_deliveries_identical_after_timestamp_normalization": true,
                "session_and_turn_match": true,
            });
            let payloads: Vec<Value> = evidence
                .recorder_events
                .iter()
                .filter(|e| e.kind == RecorderEventKind::Lifecycle)
                .map(|e| e.payload.clone())
                .collect();
            let normalized: Vec<Value> = payloads
                .iter()
                .map(|p| {
                    let mut copy = p.clone();
                    if let Some(map) = copy.as_object_mut() {
                        map.remove("timestamp");
                    }
                    copy
                })
                .collect();
            let identical = normalized.windows(2).all(|w| w[0] == w[1]);
            let bound_ok = payloads.iter().all(|p| {
                p.get("session_id").and_then(Value::as_str) == Some(evidence.session_id.as_str())
                    && match &evidence.turn_id {
                        Some(turn) => p.get("turn_id").and_then(Value::as_str) == Some(turn),
                        None => false,
                    }
            });
            let actual = json!({
                "at_least_one_completion": !payloads.is_empty(),
                "all_deliveries_identical_after_timestamp_normalization": identical,
                "session_and_turn_match": bound_ok,
                "deliveries": payloads,
            });
            let passed = !payloads.is_empty() && identical && bound_ok;
            (passed, expected, actual, vec!["lifecycle-events.json"])
        }

        "router.generations_consumed" => {
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

        "target.calls" => {
            let function_id = params
                .get("function_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let want_count = params.get("count").and_then(Value::as_u64).unwrap_or(0);
            let calls: Vec<&RecorderEventV1> = evidence
                .recorder_events
                .iter()
                .filter(|e| e.kind == RecorderEventKind::TargetCall && e.function_id == function_id)
                .collect();
            let payload_ok = match params.get("payload") {
                Some(want) => calls.iter().all(|c| &c.payload == want),
                None => true,
            };
            let expected = Value::Object(params.clone());
            let actual = json!({
                "count": calls.len(),
                "payloads": calls.iter().map(|c| c.payload.clone()).collect::<Vec<_>>(),
            });
            let passed = calls.len() as u64 == want_count && payload_ok;
            (passed, expected, actual, vec!["target-calls.json"])
        }

        unknown => (
            false,
            json!({ "error": "unknown invariant id" }),
            json!({ "id": unknown }),
            vec![],
        ),
    }
}

fn flag(params: &Map<String, Value>, key: &str) -> bool {
    params.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// Object-subset equality: every expected member equals the actual member
/// (arrays compare positionally-exact here — the grader's expectations are
/// literal).
fn subset_eq(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(exp), Value::Object(act)) => exp
            .iter()
            .all(|(k, v)| act.get(k).is_some_and(|a| subset_eq(v, a))),
        (e, a) => e == a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::script::SchemaVersion1;

    fn event(kind: RecorderEventKind, function_id: &str, payload: Value) -> RecorderEventV1 {
        RecorderEventV1 {
            schema_version: SchemaVersion1::V1,
            run_id: "r".into(),
            sequence: 1,
            kind,
            function_id: function_id.into(),
            payload,
            received_at: "2026-07-15T00:00:00Z".into(),
        }
    }

    fn base_evidence() -> Evidence {
        Evidence {
            session_id: "s_1".into(),
            turn_id: Some("t_1".into()),
            send_response: Some(json!({ "session_id": "s_1", "turn_id": "t_1", "accepted": true })),
            status: json!({ "status": "completed", "pending_function_calls": [], "children": [] }),
            transcript: vec![],
            generations_consumed: 1,
            generations_total: 1,
            recorder_events: vec![],
        }
    }

    fn spec(id: &str, params: Value) -> InvariantSpecV1 {
        InvariantSpecV1 {
            id: id.into(),
            parameters: params.as_object().cloned().unwrap_or_default(),
        }
    }

    #[test]
    fn send_flags_normalize_absent_to_false() {
        let evidence = base_evidence();
        let results = grade(
            &[spec(
                "send.flags",
                json!({ "merged": false, "queued": false, "deduplicated": false }),
            )],
            &evidence,
        );
        assert!(results[0].passed, "{:?}", results[0]);
    }

    /// Spec-mandated: the duplicate-detection rule is unit-tested with
    /// synthetic evidence containing two target calls.
    #[test]
    fn target_calls_fails_on_duplicate_execution() {
        let mut evidence = base_evidence();
        evidence.recorder_events = vec![
            event(
                RecorderEventKind::TargetCall,
                "r::record",
                json!({ "value": "expected" }),
            ),
            event(
                RecorderEventKind::TargetCall,
                "r::record",
                json!({ "value": "expected" }),
            ),
        ];
        let results = grade(
            &[spec(
                "target.calls",
                json!({ "function_id": "r::record", "count": 1, "payload": { "value": "expected" } }),
            )],
            &evidence,
        );
        assert!(!results[0].passed, "two executions must fail exactly-once");
        assert_eq!(results[0].actual["count"], 2);
    }

    #[test]
    fn lifecycle_accepts_identical_duplicates_and_rejects_conflicts() {
        let completed = json!({
            "session_id": "s_1", "turn_id": "t_1", "status": "completed", "timestamp": 1
        });
        let mut dup = completed.clone();
        dup["timestamp"] = json!(99); // identical modulo timestamp
        let mut evidence = base_evidence();
        evidence.recorder_events = vec![
            event(
                RecorderEventKind::Lifecycle,
                "conformance-recorder::lifecycle",
                completed.clone(),
            ),
            event(
                RecorderEventKind::Lifecycle,
                "conformance-recorder::lifecycle",
                dup,
            ),
        ];
        let ok = grade(
            &[spec(
                "lifecycle.completed_once",
                json!({ "allow_identical_duplicates": true }),
            )],
            &evidence,
        );
        assert!(ok[0].passed, "{:?}", ok[0]);

        let mut conflicting = completed;
        conflicting["status"] = json!("failed");
        evidence.recorder_events.push(event(
            RecorderEventKind::Lifecycle,
            "conformance-recorder::lifecycle",
            conflicting,
        ));
        let bad = grade(&[spec("lifecycle.completed_once", json!({}))], &evidence);
        assert!(!bad[0].passed, "conflicting terminals must fail");
    }

    #[test]
    fn unused_router_expectation_fails_generations_consumed() {
        let mut evidence = base_evidence();
        evidence.generations_total = 2;
        let results = grade(
            &[spec("router.generations_consumed", json!({ "count": 1 }))],
            &evidence,
        );
        assert!(!results[0].passed);
        assert_eq!(results[0].actual["unused_expectations"], 1);
    }

    #[test]
    fn unknown_invariant_id_fails_closed() {
        let results = grade(&[spec("nonsense.id", json!({}))], &base_evidence());
        assert!(!results[0].passed);
    }
}
