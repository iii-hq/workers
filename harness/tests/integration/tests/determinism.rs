//! Failure text is scrubbed before it enters stable result artifacts.

use std::collections::BTreeMap;

use harness_integration::canonical::canonical_json_pretty;
use harness_integration::evidence_data::RunEvidence;
use harness_integration::scenario::floor::{floor_failure, verify_failure};
use harness_integration::types::scenario::{Classification, IntegrationResultV1};
use harness_integration::types::script::SchemaVersion1;
use harness_integration::types::trace::{TraceEventV1, TraceEvidenceV1, TraceSpanV1, TraceTreeV1};
use serde_json::{json, Value};

fn span(
    trace_id: &str,
    span_id: &str,
    name: String,
    session_id: &str,
    turn_id: &str,
    payload: Value,
) -> TraceSpanV1 {
    TraceSpanV1 {
        trace_id: trace_id.into(),
        span_id: span_id.into(),
        parent_span_id: None,
        name,
        start_time_unix_nano: 1,
        end_time_unix_nano: 2,
        status: "ok".into(),
        status_description: None,
        attributes: BTreeMap::from([
            ("iii.session.id".into(), session_id.into()),
            ("iii.message.id".into(), turn_id.into()),
        ]),
        service_name: "integration-probe".into(),
        events: vec![TraceEventV1 {
            name: "iii.invocation.input".into(),
            timestamp_unix_nano: 1,
            attributes: BTreeMap::from([
                ("iii.payload.json".into(), payload.to_string()),
                ("iii.payload.truncated".into(), "false".into()),
            ]),
        }],
        links: Vec::new(),
        instrumentation_scope_name: None,
        instrumentation_scope_version: None,
        flags: None,
        trace_state: None,
        pending: false,
        children: Vec::new(),
    }
}

fn evidence(run_id: &str, session_id: &str, turn_id: &str) -> RunEvidence {
    let trace_id = format!("trace-{turn_id}");
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
        traces: TraceEvidenceV1::new(vec![TraceTreeV1 {
            trace_id: trace_id.clone(),
            roots: vec![
                span(
                    &trace_id,
                    "target",
                    format!("execute {run_id}::record"),
                    session_id,
                    turn_id,
                    json!({ "value": "expected" }),
                ),
                span(
                    &trace_id,
                    "lifecycle",
                    "execute integration-probe::turn-completed".into(),
                    session_id,
                    turn_id,
                    json!({
                        "session_id": session_id,
                        "turn_id": turn_id,
                        "status": "completed",
                        "terminal": true,
                        "timestamp": 1
                    }),
                ),
            ],
        }]),
        target_calls: Vec::new(),
        control: Value::Null,
        tree_sessions: Vec::new(),
        tree_statuses: Vec::new(),
        router_evidence: Value::Null,
        probe_responses: Vec::new(),
        hook_calls: Vec::new(),
    }
}

fn expects_a_mutated_payload(run: &RunEvidence) -> anyhow::Result<()> {
    let calls = run.calls("record");
    anyhow::ensure!(calls.len() == 1, "record ran {} times", calls.len());
    let payload = calls[0].payload.as_ref();
    anyhow::ensure!(
        payload == Some(&json!({ "value": "mutated" })),
        "session {} turn {}: {}::record payload {payload:?} does not match expected",
        run.session_id,
        run.turn_id.as_deref().unwrap_or("missing"),
        run.run_id,
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
        scenario_id: "INT-DET".into(),
        classification: Classification::ContractFailure,
        failure: Some(failure),
        artifacts: vec!["scenarios/INT-DET/transcript.json".into()],
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
