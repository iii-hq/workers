use serde_json::{json, Value};

use crate::types::recorder::{RecorderEventKind, RecorderEventV1};
use crate::types::scenario::InvariantSpecV1;
use crate::types::script::SchemaVersion1;

use super::{grade, Evidence};

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
        run_id: "r".into(),
        session_id: "s_1".into(),
        turn_id: Some("t_1".into()),
        send_response: Some(json!({
            "session_id": "s_1",
            "turn_id": "t_1",
            "accepted": true
        })),
        status: json!({
            "status": "completed",
            "pending_function_calls": [],
            "children": []
        }),
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
    let results = grade(
        &[spec(
            "send.flags",
            json!({ "merged": false, "queued": false, "deduplicated": false }),
        )],
        &base_evidence(),
    );
    assert!(results[0].passed, "{:?}", results[0]);
}

#[test]
fn target_calls_fails_on_duplicate_execution() {
    let mut evidence = base_evidence();
    let target = || {
        event(
            RecorderEventKind::TargetCall,
            "r::record",
            json!({ "value": "expected" }),
        )
    };
    evidence.recorder_events = vec![target(), target()];
    let results = grade(
        &[spec(
            "target.calls",
            json!({
                "function_id": "r::record",
                "count": 1,
                "payload": { "value": "expected" }
            }),
        )],
        &evidence,
    );
    assert!(!results[0].passed, "two executions must fail exactly-once");
    assert_eq!(results[0].actual["count"], 2);
}

#[test]
fn lifecycle_accepts_identical_duplicates_and_rejects_conflicts() {
    let completed = json!({
        "session_id": "s_1",
        "turn_id": "t_1",
        "status": "completed",
        "timestamp": 1
    });
    let mut duplicate = completed.clone();
    duplicate["timestamp"] = json!(99);
    let mut evidence = base_evidence();
    evidence.recorder_events = vec![
        event(
            RecorderEventKind::Lifecycle,
            "integration-recorder::lifecycle",
            completed.clone(),
        ),
        event(
            RecorderEventKind::Lifecycle,
            "integration-recorder::lifecycle",
            duplicate,
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

    let strict = grade(
        &[spec(
            "lifecycle.completed_once",
            json!({ "allow_identical_duplicates": false }),
        )],
        &evidence,
    );
    assert!(
        !strict[0].passed,
        "strict lifecycle expectation must reject duplicate delivery"
    );

    let mut conflicting = completed;
    conflicting["status"] = json!("failed");
    evidence.recorder_events.push(event(
        RecorderEventKind::Lifecycle,
        "integration-recorder::lifecycle",
        conflicting,
    ));
    let bad = grade(&[spec("lifecycle.completed_once", json!({}))], &evidence);
    assert!(!bad[0].passed, "conflicting terminals must fail");
}

#[test]
fn function_result_expectations_select_their_call_id() {
    let mut evidence = base_evidence();
    evidence.transcript = vec![
        json!({
            "message": {
                "role": "function_result",
                "function_call_id": "call-1",
                "function_id": "r::one",
                "content": []
            }
        }),
        json!({
            "message": {
                "role": "function_result",
                "function_call_id": "call-2",
                "function_id": "r::two",
                "content": []
            }
        }),
    ];
    let results = grade(
        &[
            spec(
                "transcript.function_result",
                json!({ "function_call_id": "call-1", "function_id": "r::one" }),
            ),
            spec(
                "transcript.function_result",
                json!({ "function_call_id": "call-2", "function_id": "r::two" }),
            ),
        ],
        &evidence,
    );
    assert!(results.iter().all(|result| result.passed), "{results:?}");
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

#[test]
fn known_invariants_preserve_lossless_parameter_maps() {
    let results = grade(
        &[
            spec("send.flags", json!({ "note": "ignored" })),
            spec("transcript.message_counts", json!({ "assistant": 0 })),
            spec("target.calls", json!({})),
            spec("transcript.calls_closed", json!({ "note": "ignored" })),
            spec("transcript.no_duplicates", json!({ "note": "ignored" })),
        ],
        &base_evidence(),
    );
    assert!(
        results.iter().all(|result| result.passed),
        "missing and extra parameters keep their legacy semantics: {results:?}"
    );
    assert_eq!(results[1].expected, json!({ "assistant": 0 }));
    assert_eq!(results[2].expected, json!({}));
}
