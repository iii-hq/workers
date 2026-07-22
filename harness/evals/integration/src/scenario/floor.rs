//! Runner-owned post-conditions shared by every scenario.
//!
//! The floor is not authored: after evidence collection and before the
//! scenario's `verify` function, the runner requires a completed turn, a
//! lifecycle delivered exactly once, a fully consumed script, and a clean
//! send. Any violation is a contract failure with a `floor: ` message.

use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

use serde_json::Value;

use crate::evidence_data::RunEvidence;
use crate::scenarios::VerifyFn;
use crate::types::recorder::RecorderEventKind;

const LIFECYCLE_SINK: &str = "integration-recorder::lifecycle";

/// First violated floor post-condition, if any. Messages are id-free and
/// deterministic for identical evidence shapes.
pub fn floor_failure(run: &RunEvidence) -> Option<String> {
    terminal_failure(run)
        .or_else(|| lifecycle_failure(run))
        .or_else(|| generations_failure(run))
        .or_else(|| send_flags_failure(run))
}

/// Run the scenario's `verify` function, catching panics so plain
/// `assert!`/`assert_eq!` checks surface as contract failures.
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

/// Terminal status `completed` with no pending function calls, no queued
/// messages, and no child sessions.
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

/// Lifecycle delivered exactly once: at least one delivery, all deliveries
/// identical after timestamp normalization, the private-sink contract shape,
/// bound to this run's session/turn with status `completed`, and sequenced
/// after every target call.
fn lifecycle_failure(run: &RunEvidence) -> Option<String> {
    let lifecycle = run.lifecycle_events();
    let delivered = !lifecycle.is_empty();

    let normalized: Vec<Value> = lifecycle
        .iter()
        .map(|event| {
            let mut copy = event.payload.clone();
            if let Some(map) = copy.as_object_mut() {
                map.remove("timestamp");
            }
            copy
        })
        .collect();
    let identical = normalized.windows(2).all(|window| window[0] == window[1]);

    let bound = lifecycle.iter().all(|event| {
        event.function_id == LIFECYCLE_SINK
            && event.payload.get("status").and_then(Value::as_str) == Some("completed")
            // `terminal: false` is a non-final completion (armed wake); the
            // scenarios here run single, final turns.
            && event.payload.get("terminal").and_then(Value::as_bool) == Some(true)
            && event.payload.get("session_id").and_then(Value::as_str)
                == Some(run.session_id.as_str())
            && match &run.turn_id {
                Some(turn) => event.payload.get("turn_id").and_then(Value::as_str) == Some(turn),
                None => false,
            }
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
    let shape = lifecycle.iter().all(|event| {
        let Some(map) = event.payload.as_object() else {
            return false;
        };
        map.get("timestamp").and_then(Value::as_i64).is_some()
            && map.keys().all(|key| allowed_keys.contains(key.as_str()))
    });

    let monotonic = lifecycle
        .windows(2)
        .all(|window| window[0].sequence < window[1].sequence);
    let last_target_sequence = run
        .recorder_events
        .iter()
        .filter(|event| event.kind == RecorderEventKind::TargetCall)
        .map(|event| event.sequence)
        .max()
        .unwrap_or(0);
    let follows_target_calls = lifecycle
        .first()
        .is_none_or(|event| event.sequence > last_target_sequence);
    let ordered = monotonic && follows_target_calls;

    if delivered && identical && bound && shape && ordered {
        return None;
    }
    Some(format!(
        "floor: lifecycle must be delivered exactly once (delivered: {delivered}, identical \
         deliveries: {identical}, contract shape: {shape}, session/turn/status bound: {bound}, \
         sequence order: {ordered})"
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

/// Send accepted with clean flags. Skipped only when `send_response` is
/// absent (should not happen for Direct or Observe after a successful Send).
fn send_flags_failure(run: &RunEvidence) -> Option<String> {
    let response = run.send_response.as_ref()?;
    // Absent optional flags normalize to false.
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
    use serde_json::json;

    use crate::types::recorder::RecorderEventV1;
    use crate::types::script::SchemaVersion1;

    use super::*;

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

    fn completed_lifecycle(timestamp: i64) -> Value {
        json!({
            "session_id": "s_1",
            "turn_id": "t_1",
            "status": "completed",
            "terminal": true,
            "timestamp": timestamp
        })
    }

    fn clean_evidence() -> RunEvidence {
        RunEvidence {
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
            recorder_events: vec![event(
                RecorderEventKind::Lifecycle,
                LIFECYCLE_SINK,
                completed_lifecycle(1),
            )],
        }
    }

    #[test]
    fn clean_evidence_passes_the_floor() {
        assert_eq!(floor_failure(&clean_evidence()), None);
    }

    #[test]
    fn send_flags_normalize_absent_to_false_and_skip_without_response() {
        let mut evidence = clean_evidence();
        evidence.send_response = Some(json!({ "accepted": true }));
        assert_eq!(floor_failure(&evidence), None);

        evidence.send_response = Some(json!({ "accepted": true, "queued": true }));
        assert!(floor_failure(&evidence).unwrap().contains("queued: true"));

        // No send response yet (or collect failed early); the flags check is skipped.
        evidence.send_response = None;
        assert_eq!(floor_failure(&evidence), None);
    }

    #[test]
    fn lifecycle_accepts_identical_duplicates_and_rejects_conflicts() {
        let mut evidence = clean_evidence();
        let mut duplicate = event(
            RecorderEventKind::Lifecycle,
            LIFECYCLE_SINK,
            completed_lifecycle(99),
        );
        duplicate.sequence = 2;
        evidence.recorder_events.push(duplicate);
        assert_eq!(floor_failure(&evidence), None);

        let mut conflicting = completed_lifecycle(1);
        conflicting["status"] = json!("failed");
        let mut conflicting_event =
            event(RecorderEventKind::Lifecycle, LIFECYCLE_SINK, conflicting);
        conflicting_event.sequence = 3;
        evidence.recorder_events.push(conflicting_event);
        let failure = floor_failure(&evidence).expect("conflicting terminals must fail");
        assert!(failure.starts_with("floor: lifecycle"), "{failure}");
    }

    #[test]
    fn lifecycle_rejects_wrong_sink_shape_and_order() {
        let mut evidence = clean_evidence();
        let mut target = event(
            RecorderEventKind::TargetCall,
            "r::record",
            json!({ "value": "expected" }),
        );
        target.sequence = 2;
        evidence.recorder_events[0].function_id = "wrong::sink".into();
        evidence.recorder_events.push(target);
        let failure = floor_failure(&evidence).unwrap();
        assert!(failure.contains("bound: false"), "{failure}");
        assert!(failure.contains("sequence order: false"), "{failure}");

        let mut evidence = clean_evidence();
        evidence.recorder_events[0].payload["unexpected"] = json!(true);
        let failure = floor_failure(&evidence).unwrap();
        assert!(failure.contains("contract shape: false"), "{failure}");

        let mut evidence = clean_evidence();
        evidence.recorder_events.clear();
        let failure = floor_failure(&evidence).unwrap();
        assert!(failure.contains("delivered: false"), "{failure}");
    }

    #[test]
    fn non_terminal_status_and_pending_work_fail_the_floor() {
        let mut evidence = clean_evidence();
        evidence.status = json!({
            "status": "running",
            "pending_function_calls": [],
            "children": []
        });
        let failure = floor_failure(&evidence).unwrap();
        assert!(failure.starts_with("floor: terminal"), "{failure}");

        let mut evidence = clean_evidence();
        evidence.status["pending_function_calls"] = json!(["call-1"]);
        assert!(floor_failure(&evidence)
            .unwrap()
            .contains("1 pending call(s)"));
    }

    #[test]
    fn incomplete_script_consumption_fails_the_floor() {
        let mut evidence = clean_evidence();
        evidence.generations_total = 2;
        assert_eq!(
            floor_failure(&evidence).unwrap(),
            "floor: scripted generations consumed 1 of 2"
        );
    }

    #[test]
    fn verify_errors_and_panics_map_to_contract_messages() {
        let evidence = clean_evidence();
        assert_eq!(verify_failure(|_| Ok(()), &evidence), None);
        assert_eq!(
            verify_failure(
                |_| Err(anyhow::anyhow!("payload mismatch").context("record call")),
                &evidence
            )
            .unwrap(),
            "verify: record call: payload mismatch"
        );
        let panicking = verify_failure(|_| panic!("expected 1 call, got 2"), &evidence).unwrap();
        assert_eq!(panicking, "verify panicked: expected 1 call, got 2");
    }
}
