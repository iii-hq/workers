//! Runner-owned post-conditions shared by every scenario.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};

use serde_json::Value;

use crate::evidence_data::RunEvidence;
use crate::probe::LIFECYCLE_FUNCTION_ID;
use crate::scenarios::VerifyFn;
use crate::types::trace::strip_engine_fields;

/// First violated floor post-condition, if any.
pub fn floor_failure(run: &RunEvidence) -> Option<String> {
    floor_failure_for_turns(run, 1)
}

pub fn floor_failure_for_turns(
    run: &RunEvidence,
    expected_terminal_turns: usize,
) -> Option<String> {
    terminal_failure(run)
        .or_else(|| trace_failure(run, expected_terminal_turns))
        .or_else(|| lifecycle_failure(run, expected_terminal_turns))
        .or_else(|| generations_failure(run))
        .or_else(|| send_flags_failure(run))
}

/// Run the scenario's `verify` function, catching assertion panics.
pub fn verify_failure(verify: VerifyFn, run: &RunEvidence) -> Option<String> {
    match catch_unwind(AssertUnwindSafe(|| verify(run))) {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!("verify: {error:#}")),
        Err(panic) => Some(format!("verify panicked: {}", panic_text(&*panic))),
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        text
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text
    } else {
        "non-string panic payload"
    }
}

/// Terminal status `completed` with no pending calls, queued messages, or children.
fn terminal_failure(run: &RunEvidence) -> Option<String> {
    let status = run.status.get("status").and_then(Value::as_str);
    let pending = status_list_len(run, "pending_function_calls");
    let queued = status_list_len(run, "queued");
    let children = status_list_len(run, "children");
    if status == Some("completed") && pending == 0 && queued == 0 && children == 0 {
        return None;
    }
    Some(format!(
        "floor: terminal status must be completed with nothing pending; got status {status:?}, \
         {pending} pending call(s), {queued} queued message(s), {children} child session(s)"
    ))
}

fn status_list_len(run: &RunEvidence, key: &str) -> usize {
    run.status
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn trace_failure(run: &RunEvidence, expected_terminal_turns: usize) -> Option<String> {
    let summary = &run.traces.summary;
    let trace_count = summary.trace_count == expected_terminal_turns;
    let turn_count = summary.turn_ids.len() == expected_terminal_turns;
    let complete = summary.pending_span_count == 0;
    let clean = summary.error_count == 0;
    let latest_bound = summary.turn_ids.last().map(String::as_str) == run.turn_id.as_deref();
    if trace_count && turn_count && complete && clean && latest_bound {
        return None;
    }
    Some(format!(
        "floor: traces must cover {expected_terminal_turns} completed clean turn(s) (trace count: \
         {}, turn count: {}, pending spans: {}, error spans: {}, latest turn bound: {latest_bound})",
        summary.trace_count,
        summary.turn_ids.len(),
        summary.pending_span_count,
        summary.error_count
    ))
}

/// Validate `harness::turn-completed` from the lifecycle sink's trace events.
fn lifecycle_failure(run: &RunEvidence, expected_terminal_turns: usize) -> Option<String> {
    let lifecycle_name = format!("execute {LIFECYCLE_FUNCTION_ID}");
    let lifecycle: Vec<Option<Value>> = run
        .spans_named(&lifecycle_name)
        .into_iter()
        .map(|span| {
            span.invocation_input().map(|mut payload| {
                strip_engine_fields(&mut payload);
                payload
            })
        })
        .collect();
    let delivered = !lifecycle.is_empty();
    let complete_payloads = lifecycle.iter().all(Option::is_some);
    let payloads = lifecycle
        .iter()
        .filter_map(Option::as_ref)
        .collect::<Vec<_>>();

    let normalized = payloads
        .iter()
        .map(|payload| {
            let mut copy = (*payload).clone();
            if let Some(map) = copy.as_object_mut() {
                map.remove("timestamp");
            }
            copy
        })
        .collect::<Vec<_>>();
    let mut turns = BTreeMap::<String, Value>::new();
    let mut consistent = true;
    for payload in &normalized {
        let Some(turn_id) = payload.get("turn_id").and_then(Value::as_str) else {
            consistent = false;
            continue;
        };
        match turns.get(turn_id) {
            Some(previous) if previous != payload => consistent = false,
            Some(_) => {}
            None => {
                turns.insert(turn_id.to_string(), payload.clone());
            }
        }
    }
    let turn_count = turns.len() == expected_terminal_turns;
    let bound = payloads.iter().all(|payload| {
        payload.get("status").and_then(Value::as_str) == Some("completed")
            && payload.get("terminal").and_then(Value::as_bool) == Some(true)
            && payload.get("session_id").and_then(Value::as_str) == Some(run.session_id.as_str())
            && payload
                .get("turn_id")
                .and_then(Value::as_str)
                .is_some_and(|turn_id| run.traces.summary.turn_ids.iter().any(|id| id == turn_id))
    });

    let allowed_keys = BTreeSet::from([
        "session_id",
        "terminal",
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
    let shape = payloads.iter().all(|payload| {
        let Some(map) = payload.as_object() else {
            return false;
        };
        map.get("timestamp").and_then(Value::as_i64).is_some()
            && map.keys().all(|key| allowed_keys.contains(key.as_str()))
    });

    if delivered && complete_payloads && turn_count && consistent && bound && shape {
        return None;
    }
    Some(format!(
        "floor: lifecycle traces must cover {expected_terminal_turns} terminal turn(s) \
         (delivered: {delivered}, complete payloads: {complete_payloads}, expected turn count: \
         {turn_count}, consistent retries: {consistent}, contract shape: {shape}, \
         session/turn/status bound: {bound})"
    ))
}

/// Every scripted generation consumed, none extra.
fn generations_failure(run: &RunEvidence) -> Option<String> {
    (run.generations_consumed != run.generations_total).then(|| {
        format!(
            "floor: scripted generations consumed {} of {}",
            run.generations_consumed, run.generations_total
        )
    })
}

/// Send accepted with clean flags. Skipped for Playground-owned sends.
fn send_flags_failure(run: &RunEvidence) -> Option<String> {
    let response = run.send_response.as_ref()?;
    let flag = |key: &str| response.get(key) == Some(&Value::Bool(true));
    let (accepted, merged, queued, deduplicated) = (
        flag("accepted"),
        flag("merged"),
        flag("queued"),
        flag("deduplicated"),
    );
    if accepted && !merged && !queued && !deduplicated {
        return None;
    }
    Some(format!(
        "floor: send must be accepted with clean flags; got accepted: {accepted}, merged: \
         {merged}, queued: {queued}, deduplicated: {deduplicated}"
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use crate::types::trace::{TraceEventV1, TraceEvidenceV1, TraceSpanV1, TraceTreeV1};

    use super::*;

    fn lifecycle_span(turn_id: &str, payload: Value) -> TraceSpanV1 {
        TraceSpanV1 {
            trace_id: format!("trace-{turn_id}"),
            span_id: format!("span-{turn_id}"),
            parent_span_id: None,
            name: format!("execute {LIFECYCLE_FUNCTION_ID}"),
            start_time_unix_nano: if turn_id == "turn-1" { 1 } else { 2 },
            end_time_unix_nano: 3,
            status: "ok".into(),
            status_description: None,
            attributes: BTreeMap::from([
                ("iii.session.id".into(), "session-1".into()),
                ("iii.message.id".into(), turn_id.into()),
            ]),
            service_name: "integration-probe".into(),
            events: vec![TraceEventV1 {
                name: "iii.invocation.input".into(),
                timestamp_unix_nano: 2,
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

    fn completed_lifecycle(turn_id: &str, timestamp: i64) -> Value {
        json!({
            "session_id": "session-1",
            "turn_id": turn_id,
            "status": "completed",
            "terminal": true,
            "timestamp": timestamp
        })
    }

    fn clean_evidence() -> RunEvidence {
        RunEvidence {
            run_id: "run-1".into(),
            session_id: "session-1".into(),
            turn_id: Some("turn-1".into()),
            send_response: Some(json!({
                "accepted": true,
                "merged": false,
                "queued": false,
                "deduplicated": false
            })),
            status: json!({
                "status": "completed",
                "pending_function_calls": [],
                "queued": [],
                "children": []
            }),
            transcript: Vec::new(),
            generations_consumed: 1,
            generations_total: 1,
            traces: TraceEvidenceV1::new(vec![TraceTreeV1 {
                trace_id: "trace-turn-1".into(),
                roots: vec![lifecycle_span("turn-1", completed_lifecycle("turn-1", 1))],
            }]),
        }
    }

    #[test]
    fn clean_evidence_passes() {
        assert_eq!(floor_failure(&clean_evidence()), None);
    }

    #[test]
    fn pending_or_error_spans_fail_the_trace_floor() {
        let mut pending = clean_evidence();
        pending.traces.traces[0].roots[0].pending = true;
        pending.traces = TraceEvidenceV1::new(pending.traces.traces);
        assert!(floor_failure(&pending)
            .unwrap()
            .contains("pending spans: 1"));

        let mut error = clean_evidence();
        error.traces.traces[0].roots[0].status = "error".into();
        error.traces = TraceEvidenceV1::new(error.traces.traces);
        assert!(floor_failure(&error).unwrap().contains("error spans: 1"));
    }

    #[test]
    fn consistent_lifecycle_retries_are_accepted() {
        let mut evidence = clean_evidence();
        let duplicate = lifecycle_span("turn-1", completed_lifecycle("turn-1", 2));
        evidence.traces.traces[0].roots.push(duplicate);
        evidence.traces = TraceEvidenceV1::new(evidence.traces.traces);
        assert_eq!(floor_failure(&evidence), None);
    }

    #[test]
    fn conflicting_lifecycle_retries_fail() {
        let mut evidence = clean_evidence();
        let mut conflicting = completed_lifecycle("turn-1", 2);
        conflicting["status"] = json!("failed");
        evidence.traces.traces[0]
            .roots
            .push(lifecycle_span("turn-1", conflicting));
        evidence.traces = TraceEvidenceV1::new(evidence.traces.traces);
        assert!(floor_failure(&evidence)
            .unwrap()
            .contains("consistent retries: false"));
    }

    #[test]
    fn multiple_turns_are_bound_in_trace_order() {
        let mut evidence = clean_evidence();
        evidence.turn_id = Some("turn-2".into());
        evidence.traces.traces.push(TraceTreeV1 {
            trace_id: "trace-turn-2".into(),
            roots: vec![lifecycle_span("turn-2", completed_lifecycle("turn-2", 2))],
        });
        evidence.traces = TraceEvidenceV1::new(evidence.traces.traces);
        assert_eq!(floor_failure_for_turns(&evidence, 2), None);
    }

    #[test]
    fn verify_panics_become_failures() {
        fn panics(_: &RunEvidence) -> anyhow::Result<()> {
            panic!("broken assertion")
        }
        assert_eq!(
            verify_failure(panics, &clean_evidence()).as_deref(),
            Some("verify panicked: broken assertion")
        );
    }
}
