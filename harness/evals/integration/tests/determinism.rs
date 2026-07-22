//! `--repeat` requires byte-identical stable results. Failure text is the
//! only run-dependent part of `result.json`, so the scrub must replace every
//! run-scoped id with its placeholder before the result is persisted.

use harness_integration::canonical::canonical_json_pretty;
use harness_integration::evidence_data::RunEvidence;
use harness_integration::scenario::floor::{floor_failure, verify_failure};
use harness_integration::types::recorder::{RecorderEventKind, RecorderEventV1};
use harness_integration::types::scenario::{Classification, IntegrationResultV1};
use harness_integration::types::script::SchemaVersion1;
use serde_json::json;

fn evidence(run_id: &str, session_id: &str, turn_id: &str) -> RunEvidence {
    let event = |sequence, kind, function_id: &str, payload| RecorderEventV1 {
        schema_version: SchemaVersion1::V1,
        run_id: run_id.to_string(),
        sequence,
        kind,
        function_id: function_id.to_string(),
        payload,
        received_at: "2026-07-18T00:00:00Z".to_string(),
    };
    RunEvidence {
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
                    "terminal": true,
                    "timestamp": 1
                }),
            ),
        ],
    }
}

/// A scenario-style verify whose failure message embeds every run-scoped id
/// the evidence exposes.
fn expects_a_mutated_payload(run: &RunEvidence) -> anyhow::Result<()> {
    let calls = run.calls("record");
    anyhow::ensure!(calls.len() == 1, "record ran {} times", calls.len());
    let payload = &calls[0].payload;
    anyhow::ensure!(
        payload == &json!({ "value": "mutated" }),
        "{}::record payload {payload} != {{\"value\":\"mutated\"}}",
        run.run_id
    );
    Ok(())
}

fn scrubbed_failure(run_id: &str, session_id: &str, turn_id: &str) -> String {
    let run = evidence(run_id, session_id, turn_id);
    assert_eq!(floor_failure(&run), None, "fixture must pass the floor");
    let failure = verify_failure(expects_a_mutated_payload, &run).expect("verify must fail");
    run.scrub(&failure)
}

#[test]
fn scrubbed_failure_strings_ignore_execution_ids() {
    let first = scrubbed_failure("run-a", "session-a", "turn-a");
    let second = scrubbed_failure("run-b", "session-b", "turn-b");
    assert_eq!(first, second);
    for placeholder in ["{{run_id}}", "{{session_id}}", "{{turn_id}}"] {
        assert!(first.contains(placeholder), "{first}");
    }
    for raw in ["run-a", "session-a", "turn-a"] {
        assert!(!first.contains(raw), "{first}");
    }
}

#[test]
fn scrubbed_stable_results_are_byte_identical() {
    let result = |failure| IntegrationResultV1 {
        schema_version: SchemaVersion1::V1,
        scenario_id: "E2E-DET".into(),
        classification: Classification::ContractFailure,
        failure: Some(failure),
        artifacts: vec!["scenarios/E2E-DET/transcript.json".into()],
    };

    let first = canonical_json_pretty(
        &serde_json::to_value(result(scrubbed_failure("run-a", "session-a", "turn-a"))).unwrap(),
    );
    let second = canonical_json_pretty(
        &serde_json::to_value(result(scrubbed_failure("run-b", "session-b", "turn-b"))).unwrap(),
    );
    assert_eq!(first, second);
    assert!(first.contains("{{turn_id}}"));
    assert!(!first.contains("turn-a"));
}
