use harness_integration::canonical::canonical_json_pretty;
use harness_integration::grader::{grade, Evidence};
use harness_integration::types::recorder::{RecorderEventKind, RecorderEventV1};
use harness_integration::types::scenario::{Classification, IntegrationResultV1, InvariantSpecV1};
use harness_integration::types::script::SchemaVersion1;
use serde_json::json;

fn evidence(run_id: &str, session_id: &str, turn_id: &str, timestamp: i64) -> Evidence {
    let event = |sequence, kind, function_id: &str, payload| RecorderEventV1 {
        schema_version: SchemaVersion1::V1,
        run_id: run_id.to_string(),
        sequence,
        kind,
        function_id: function_id.to_string(),
        payload,
        received_at: format!("2026-07-18T00:00:{timestamp:02}Z"),
    };
    Evidence {
        run_id: run_id.into(),
        session_id: session_id.into(),
        turn_id: Some(turn_id.into()),
        send_response: Some(json!({
            "session_id": session_id,
            "turn_id": turn_id,
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
        recorder_events: vec![
            event(
                1,
                RecorderEventKind::TargetCall,
                &format!("{run_id}::record"),
                json!({
                    "session_id": session_id,
                    "turn_id": turn_id,
                    "timestamp": timestamp,
                    "value": "expected"
                }),
            ),
            event(
                2,
                RecorderEventKind::Lifecycle,
                "integration-recorder::lifecycle",
                json!({
                    "session_id": session_id,
                    "turn_id": turn_id,
                    "status": "completed",
                    "timestamp": timestamp
                }),
            ),
        ],
    }
}

fn specs(run_id: &str) -> Vec<InvariantSpecV1> {
    serde_json::from_value(json!([
        {
            "id": "target.calls",
            "parameters": {
                "function_id": format!("{run_id}::record"),
                "count": 1,
                "payload_subset": { "value": "expected" }
            }
        },
        {
            "id": "lifecycle.completed_once",
            "parameters": { "allow_identical_duplicates": true }
        }
    ]))
    .unwrap()
}

#[test]
fn stable_grades_ignore_execution_ids_and_timestamps() {
    let first_invariants = grade(
        &specs("run-a"),
        &evidence("run-a", "session-a", "turn-a", 1),
    );
    let second_invariants = grade(
        &specs("run-b"),
        &evidence("run-b", "session-b", "turn-b", 9),
    );
    assert!(first_invariants.iter().all(|result| result.passed));
    assert!(second_invariants.iter().all(|result| result.passed));

    let result = |invariants| IntegrationResultV1 {
        schema_version: SchemaVersion1::V1,
        scenario_id: "E2E-DET".into(),
        classification: Classification::Pass,
        invariants,
        artifacts: vec!["scenarios/E2E-DET/invariants.json".into()],
    };

    let first = canonical_json_pretty(&serde_json::to_value(result(first_invariants)).unwrap());
    let second = canonical_json_pretty(&serde_json::to_value(result(second_invariants)).unwrap());
    assert_eq!(first, second);
    assert!(first.contains("{{run_id}}"));
    assert!(!first.contains("run-a"));
}

#[test]
fn failed_grades_normalize_ids_embedded_in_diagnostics() {
    let make = |run_id: &str, session_id: &str, turn_id: &str| {
        let mut evidence = evidence(run_id, session_id, turn_id, 1);
        let entry_id = format!("e_{turn_id}_1_assistant");
        evidence.transcript = vec![
            json!({ "entry_id": entry_id }),
            json!({ "entry_id": format!("e_{turn_id}_1_assistant") }),
        ];
        evidence.recorder_events[0].payload["derived_id"] =
            json!(format!("target-{turn_id}-result"));
        let specs = vec![
            InvariantSpecV1 {
                id: "transcript.no_duplicates".into(),
                parameters: Default::default(),
            },
            serde_json::from_value(json!({
                "id": "target.calls",
                "parameters": {
                    "function_id": format!("{run_id}::record"),
                    "count": 1,
                    "payload_subset": { "value": "wrong" }
                }
            }))
            .unwrap(),
        ];
        grade(&specs, &evidence)
    };

    let first =
        canonical_json_pretty(&serde_json::to_value(make("run-a", "session-a", "turn-a")).unwrap());
    let second =
        canonical_json_pretty(&serde_json::to_value(make("run-b", "session-b", "turn-b")).unwrap());
    assert_eq!(first, second);
    assert!(first.contains("{{turn_id}}"));
}
