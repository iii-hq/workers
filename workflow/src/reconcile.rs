use serde_json::{json, Value};

use crate::{
    error::WorkflowError,
    ids::node_result_key,
    state,
    types::{NodeState, WorkflowRunRecord},
};

// ---------------------------------------------------------------------------
// NodeOutcome
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum NodeOutcome {
    StillRunning,
    Done(Value),
    Failed(String),
    Cancelled,
}

// ---------------------------------------------------------------------------
// Result size cap
// ---------------------------------------------------------------------------

/// Max serialized bytes for a single node result. A result flows verbatim into
/// downstream nodes' prompts AND is loaded into memory on every tick
/// (`load_done_results`), so an unbounded blob is a memory / cost / provider-reject
/// hazard. Passing megabytes inline between agents is an anti-pattern (pass a
/// reference/key instead), so an oversized result is REJECTED (the node fails)
/// rather than truncated — truncation would yield malformed JSON that derails
/// gather_input / fanout downstream. Safety ceiling, tunable.
const MAX_RESULT_BYTES: usize = 1 << 20; // 1 MiB

/// `Some(len)` if the serialized result exceeds the cap, else `None`. Pure.
fn oversized_result(result: &Value) -> Option<usize> {
    let len = serde_json::to_string(result).map(|s| s.len()).unwrap_or(0);
    (len > MAX_RESULT_BYTES).then_some(len)
}

// ---------------------------------------------------------------------------
// classify_terminal (pure)
// ---------------------------------------------------------------------------

/// Classify a harness turn status into a `NodeOutcome`.
///
/// Rules (in priority order):
/// 1. `"completed"` + `result_error` present → `Failed` (contract violation)
/// 2. `"completed"` (no error) → `Done(result)` if within `MAX_RESULT_BYTES`,
///    else `Failed` (oversized output rejected)
/// 3. `"failed"` → `Failed(result_error.unwrap_or("failed"))`
/// 4. `"cancelled"` → `Cancelled`
/// 5. anything else → `StillRunning`
pub fn classify_terminal(
    status: &str,
    result: Option<Value>,
    result_error: Option<String>,
) -> NodeOutcome {
    match status {
        "completed" => {
            if let Some(err) = result_error {
                NodeOutcome::Failed(err)
            } else {
                let res = result.unwrap_or(Value::Null);
                match oversized_result(&res) {
                    Some(len) => NodeOutcome::Failed(format!(
                        "node result is {len} bytes, exceeds the cap of {MAX_RESULT_BYTES} \
                         (oversized output rejected; pass a reference, not the blob)"
                    )),
                    None => NodeOutcome::Done(res),
                }
            }
        }
        "failed" => NodeOutcome::Failed(result_error.unwrap_or_else(|| "failed".into())),
        "cancelled" => NodeOutcome::Cancelled,
        _ => NodeOutcome::StillRunning,
    }
}

// ---------------------------------------------------------------------------
// reconcile_run
// ---------------------------------------------------------------------------

/// For each `Running` checkpoint that has a `session_id`, poll
/// `harness::status` and advance the checkpoint to its terminal state (Done /
/// Failed / Cancelled).  Skips unknown sessions (null response) and sessions
/// still awaiting a wake (`expects_wake` — the run's outcome arrives on a
/// later turn in the same session).
pub async fn reconcile_run(
    deps: &crate::functions::Deps,
    record: &mut WorkflowRunRecord,
) -> Result<(), WorkflowError> {
    // Collect the (uid, session_id) tuples for all Running nodes so we don't
    // hold a &mut reference across .await points.
    let running: Vec<(String, String)> = record
        .nodes
        .iter()
        .filter(|(_, cp)| cp.state == NodeState::Running)
        .filter_map(|(uid, cp)| cp.session_id.as_ref().map(|sid| (uid.clone(), sid.clone())))
        .collect();

    let timeout_ms = deps.cfg().await.dispatch_timeout_ms;
    let now = deps.now_ms();

    for (uid, session_id) in running {
        // Poll harness::status. A transient/timeout error polling ONE node must NOT
        // abort reconciliation of the rest of the run (which previously bubbled up,
        // failed the whole tick, and dead-lettered). Log and move on — the node
        // stays Running and is re-polled next tick; the timeout sweep is the backstop.
        let resp = match deps
            .iii
            .trigger(iii_sdk::protocol::TriggerRequest {
                function_id: "harness::status".into(),
                payload: json!({ "session_id": session_id }),
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!(
                    run_id = %record.run_id,
                    node = %uid,
                    session_id = %session_id,
                    error = %e,
                    "reconcile: harness::status poll failed; leaving node Running"
                );
                continue;
            }
        };

        // Null → unknown session, still running
        if resp.is_null() {
            continue;
        }

        // The node's outcome is the SESSION's terminal turn, not necessarily
        // the turn harness::send started: a one-way agent completes its first
        // turn as a wiring ack (with a wake armed) and a later
        // notification-woken turn in the same session carries the real result.
        // `expects_wake` mirrors the `terminal` flag on turn-completed; while
        // it holds, the node stays Running whatever the current turn reports.
        // Node sessions are deterministic per-attempt ids (`…@r<n>`), so a
        // later turn in this session is still this node — the old exact
        // turn-id guard would have skipped the woken turns forever.
        if resp
            .get("expects_wake")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            continue;
        }

        let status = resp
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("running");

        let result = resp.get("result").cloned();
        let result_error = resp
            .get("result_error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        match classify_terminal(status, result, result_error) {
            NodeOutcome::StillRunning => {}
            NodeOutcome::Done(res) => {
                // Persist the result blob
                state::put_node_result(&deps.iii, &record.run_id, &uid, &res).await?;
                // Update checkpoint
                if let Some(cp) = record.nodes.get_mut(&uid) {
                    cp.result_ref = Some(node_result_key(&record.run_id, &uid));
                    cp.state = NodeState::Done;
                    let dur = cp
                        .pending_at
                        .map(|p| (now - p).max(0) as f64)
                        .unwrap_or(0.0);
                    crate::telemetry::record_node_terminal(true, dur);
                }
            }
            NodeOutcome::Failed(err) => {
                if let Some(cp) = record.nodes.get_mut(&uid) {
                    cp.result_error = Some(err);
                    cp.state = NodeState::Failed;
                    let dur = cp
                        .pending_at
                        .map(|p| (now - p).max(0) as f64)
                        .unwrap_or(0.0);
                    crate::telemetry::record_node_terminal(false, dur);
                }
            }
            NodeOutcome::Cancelled => {
                if let Some(cp) = record.nodes.get_mut(&uid) {
                    cp.state = NodeState::Cancelled;
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn completed_without_error_is_done() {
        let outcome = classify_terminal("completed", Some(json!({"answer": 42})), None);
        assert_eq!(outcome, NodeOutcome::Done(json!({"answer": 42})));
    }

    #[test]
    fn completed_with_result_error_is_failed() {
        let outcome = classify_terminal(
            "completed",
            Some(json!({"answer": 42})),
            Some("schema validation failed".into()),
        );
        match outcome {
            NodeOutcome::Failed(msg) => assert_eq!(msg, "schema validation failed"),
            other => panic!("expected Failed, got {:?}", other),
        }
    }

    #[test]
    fn failed_and_cancelled_map_through() {
        // "failed" with an explicit error message
        let outcome_failed = classify_terminal("failed", None, Some("something broke".into()));
        match outcome_failed {
            NodeOutcome::Failed(msg) => assert_eq!(msg, "something broke"),
            other => panic!("expected Failed, got {:?}", other),
        }

        // "failed" with no error message falls back to "failed"
        let outcome_failed_default = classify_terminal("failed", None, None);
        match outcome_failed_default {
            NodeOutcome::Failed(msg) => assert_eq!(msg, "failed"),
            other => panic!("expected Failed(\"failed\"), got {:?}", other),
        }

        // "cancelled"
        let outcome_cancelled = classify_terminal("cancelled", None, None);
        assert_eq!(outcome_cancelled, NodeOutcome::Cancelled);
    }

    #[test]
    fn non_terminal_is_still_running() {
        for status in &["running", "awaiting_nodes", "queued", "unknown", ""] {
            let outcome = classify_terminal(status, None, None);
            assert_eq!(
                outcome,
                NodeOutcome::StillRunning,
                "expected StillRunning for status {:?}",
                status
            );
        }
    }

    #[test]
    fn oversized_result_helper_flags_only_over_cap() {
        assert!(oversized_result(&json!({"k": "v"})).is_none());
        let big = Value::String("x".repeat(MAX_RESULT_BYTES + 1));
        assert!(oversized_result(&big).is_some());
    }

    #[test]
    fn completed_oversized_result_is_failed_not_done() {
        // A completed node whose result blows past the cap fails instead of being
        // stored + fed downstream / loaded into memory every tick.
        let big = Value::String("x".repeat(MAX_RESULT_BYTES + 10));
        match classify_terminal("completed", Some(big), None) {
            NodeOutcome::Failed(msg) => assert!(
                msg.contains("exceeds the cap"),
                "message should name the cap, got: {msg}"
            ),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn completed_modest_result_unaffected_by_cap() {
        let outcome = classify_terminal("completed", Some(json!({"k": "v"})), None);
        assert_eq!(outcome, NodeOutcome::Done(json!({"k": "v"})));
    }
}
