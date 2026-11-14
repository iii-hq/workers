use serde_json::{json, Value};

use crate::{
    error::WorkflowError,
    ids::node_result_key,
    state,
    types::{NodeState, WorkflowDef, WorkflowRunRecord},
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
pub(crate) const MAX_RESULT_BYTES: usize = 1 << 20; // 1 MiB

/// Milliseconds between a node's first `harness::status` reply that reads finished
/// and the confirming poll it settles from. Two harness gaps show a parked node's
/// parking turn as `completed` with no wake pending: a wake claimed before its turn
/// is seeded, and a queued wake drained after the parked turn's completed write. By
/// the second poll the woken turn is running, or finished with its own result.
const SETTLE_CONFIRM_MS: u64 = 2_000;

/// `Some(len)` if the serialized result exceeds the cap, else `None`. Pure.
pub(crate) fn oversized_result(result: &Value) -> Option<usize> {
    let len = serde_json::to_string(result).map(|s| s.len()).unwrap_or(0);
    (len > MAX_RESULT_BYTES).then_some(len)
}

// ---------------------------------------------------------------------------
// JSON node results
// ---------------------------------------------------------------------------

/// A JSON node's value from its harness turn result. The node runs under a text
/// contract, so the result is the final message's text: parsed as JSON (one
/// fenced code block around it is tolerated) and checked against `schema`. A
/// non-string result (a turn sent under the harness's JSON contract before this
/// worker sent text contracts) is checked as it is. Pure.
pub(crate) fn json_node_result(schema: Option<&Value>, result: Value) -> Result<Value, String> {
    let value = match result {
        Value::String(text) => parse_json_text(&text)?,
        other => other,
    };
    validate_value(schema, &value)?;
    Ok(value)
}

/// `value` checked against `schema` (none: anything passes). Pure.
pub(crate) fn validate_value(schema: Option<&Value>, value: &Value) -> Result<(), String> {
    let Some(schema) = schema else {
        return Ok(());
    };
    let compiled = jsonschema::JSONSchema::compile(schema)
        .map_err(|e| format!("the node's output schema does not compile: {e}"))?;
    if let Err(errors) = compiled.validate(value) {
        let errors: Vec<String> = errors.take(10).map(|e| e.to_string()).collect();
        return Err(format!(
            "the result does not match the node's output schema: {}",
            errors.join("; ")
        ));
    }
    Ok(())
}

/// A JSON output contract's schema; `schema: null` is no schema, as the harness
/// reads it. Pure.
pub(crate) fn output_schema(output: &Value) -> Option<&Value> {
    output.get("schema").filter(|schema| !schema.is_null())
}

/// Why an answer does not settle a JSON node with contract `output`, the schema
/// repeated: the opening that carried it may be compacted away by then. Pure.
pub(crate) fn correction_reason(output: &Value, error: String) -> String {
    match output_schema(output) {
        Some(schema) => format!("{error}{SCHEMA_MARKER}{schema}"),
        None => error,
    }
}

/// What [`correction_reason`] puts between the cause and the schema.
const SCHEMA_MARKER: &str = ". Required JSON Schema: ";

/// A failed node's error as stored: without the schema [`correction_reason`]
/// appended, which is for the node's re-prompt. Node errors are joined into the
/// run's error and posted to the caller's chat. Pure.
fn stored_error(error: &str) -> &str {
    error
        .split_once(SCHEMA_MARKER)
        .map_or(error, |(cause, _)| cause)
}

/// The text as JSON, unwrapping one ```` ``` ```` fence (with or without an info
/// string such as `json`). Pure.
fn parse_json_text(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    let body = trimmed
        .strip_prefix("```")
        .and_then(|rest| rest.strip_suffix("```"))
        .map(|fenced| fenced.split_once('\n').map_or(fenced, |(_, body)| body))
        .unwrap_or(trimmed);
    serde_json::from_str(body.trim()).map_err(|e| format!("the answer is not a JSON value ({e})"))
}

/// A finished node's value: a JSON node's text result parsed and checked against
/// its schema, or the [`correction_reason`] that rejects it; any other node's
/// result as it is. Pure.
pub(crate) fn json_node_outcome(
    def: &WorkflowDef,
    uid: &str,
    result: Value,
) -> Result<Value, String> {
    let base_id = uid.split('#').next().unwrap_or(uid);
    let Some(output) = def.nodes.get(base_id).and_then(|n| n.agent.json_output()) else {
        return Ok(result);
    };
    json_node_result(output_schema(output), result).map_err(|e| correction_reason(output, e))
}

/// The start of the reason a JSON node that delivered nothing is corrected with.
pub(crate) const NOT_SUBMITTED: &str = "no result was delivered with workflow-result::submit";

/// The value a JSON node delivered with `workflow-result::submit` in this
/// attempt, read back from the node's result slot; `None` when it delivered
/// nothing (or the read failed: the final message is then checked instead).
async fn submitted_result(
    deps: &crate::functions::Deps,
    record: &WorkflowRunRecord,
    uid: &str,
) -> Option<Value> {
    if !record.nodes.get(uid).is_some_and(|cp| cp.submitted) {
        return None;
    }
    state::get_node_result(&deps.iii, &record.run_id, uid)
        .await
        .inspect_err(|e| {
            tracing::warn!(run_id = %record.run_id, node = %uid, error = %e, "reconcile: submitted result unreadable; checking the final message")
        })
        .ok()
        .flatten()
}

/// Corrections reconcile sends into a JSON node's session before a rejected
/// answer fails the node; the harness's default `max_validation_retries`. A turn
/// the validate-result hook denied that often fails in the harness, so these
/// cover a turn it did not check (worker down, hook timeout).
const MAX_JSON_CORRECTIONS: u32 = 2;

/// The `harness::send` payload of correction `n` for a JSON node's session. The
/// key is deterministic, so resending after a lost reply is a harness no-op. Pure.
fn correction_payload(session_id: &str, n: u32, reason: &str) -> Value {
    json!({
        "session_id": session_id,
        "idempotency_key": format!("{session_id}#correction{n}"),
        "message": crate::functions::validate_result::RETRY_PROMPT.replace("{reason}", reason),
    })
}

/// A JSON node whose settled answer is rejected (`reason`): send a correction
/// into its session and keep it Running, or fail it once `MAX_JSON_CORRECTIONS`
/// were sent. A failed send keeps it Running for the next tick; the node's
/// timeout is the backstop.
async fn correct_json_node(
    deps: &crate::functions::Deps,
    record: &mut WorkflowRunRecord,
    uid: &str,
    reason: String,
    timeout_ms: u64,
) -> NodeOutcome {
    let run_id = record.run_id.clone();
    let Some((cp, sid)) = record
        .nodes
        .get_mut(uid)
        .and_then(|cp| cp.session_id.clone().map(|sid| (cp, sid)))
    else {
        return NodeOutcome::Failed(format!("JSON node result rejected: {reason}"));
    };
    if cp.corrections >= MAX_JSON_CORRECTIONS {
        return NodeOutcome::Failed(format!("JSON node result rejected: {reason}"));
    }
    let request = iii_sdk::CallRequest {
        function_id: "harness::send".into(),
        payload: correction_payload(&sid, cp.corrections, &reason),
        action: None,
        timeout_ms: Some(timeout_ms),
    };
    match deps.iii.call(request).await {
        Ok(_) => cp.corrections += 1,
        Err(e) => tracing::warn!(
            run_id = %run_id,
            node = %uid,
            error = %e,
            "reconcile: JSON node correction send failed; retrying next tick"
        ),
    }
    NodeOutcome::StillRunning
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

/// The outcome a node's `harness::status` reply gives it on a run stopped or not
/// (`abort`). Pure.
pub(crate) fn status_outcome(resp: &Value, abort: bool) -> NodeOutcome {
    // Null → unknown session, still running.
    //
    // The node's outcome is the SESSION's terminal turn, not necessarily
    // the turn harness::send started: a one-way agent completes its first
    // turn as a wiring ack (with a wake armed) and a later
    // notification-woken turn in the same session carries the real result.
    // `expects_wake` mirrors the `terminal` flag on turn-completed; while
    // it holds, the node stays Running whatever the current turn reports.
    // Node sessions are deterministic per-attempt ids (`…@r<n>`), so a
    // later turn in this session is still this node — the old exact
    // turn-id guard would have skipped the woken turns forever.
    // A finished reply is trusted only from the confirming poll
    // (`settle_outcomes`): in a wake gap the parking turn reads finished.
    if resp.is_null() || resp["expects_wake"].as_bool().unwrap_or(false) {
        return NodeOutcome::StillRunning;
    }
    let status = resp["status"].as_str().unwrap_or("running");
    let result_error = resp["result_error"].as_str().map(str::to_string);
    match classify_terminal(status, resp.get("result").cloned(), result_error) {
        // A stopped parked node reads its wiring ack until the harness records the cancel.
        NodeOutcome::Done(_) if abort => NodeOutcome::StillRunning,
        outcome => outcome,
    }
}

// ---------------------------------------------------------------------------
// reconcile_run
// ---------------------------------------------------------------------------

/// The caller session to ask `harness::status` when this pass cancelled an
/// `on_error: continue` node. Stopping the caller cancels every node in its tree,
/// and a tolerant node would otherwise settle as null and let its consumers fire. Pure.
fn caller_to_check<'a>(
    def: &WorkflowDef,
    record: &'a WorkflowRunRecord,
    cancelled: &[String],
) -> Option<&'a str> {
    let tolerant = cancelled
        .iter()
        .any(|uid| crate::dag::tolerant(def, uid.split('#').next().unwrap_or(uid)));
    record.caller_session_id.as_deref().filter(|_| tolerant)
}

/// For each `Running` checkpoint that has a `session_id`, poll
/// `harness::status` and advance the checkpoint to its terminal state (Done /
/// Failed / Cancelled).  Skips unknown sessions (null response) and sessions
/// still awaiting a wake (`expects_wake` — the run's outcome arrives on a
/// later turn in the same session). A finished reply is trusted only from the
/// confirming poll `SETTLE_CONFIRM_MS` later (`settle_outcomes`). A tolerant node
/// cancelled by a stop of the caller's turn aborts the run (`caller_to_check`).
pub async fn reconcile_run(
    deps: &crate::functions::Deps,
    def: &WorkflowDef,
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
    let (iii, run_id) = (&deps.iii, &record.run_id);
    // A transient/timeout error polling ONE node must NOT abort reconciliation of
    // the rest of the run (which previously bubbled up, failed the whole tick, and
    // dead-lettered). Log and move on — the node stays Running and is re-polled
    // next tick; the timeout sweep is the backstop.
    let poll = move |uid: &str, session_id: &str| {
        let (uid, session_id) = (uid.to_string(), session_id.to_string());
        async move {
            let request = iii_sdk::CallRequest {
                function_id: "harness::status".into(),
                payload: json!({ "session_id": session_id, "verbose": true }),
                action: None,
                timeout_ms: Some(timeout_ms),
            };
            iii.call(request)
                .await
                .inspect_err(|e| {
                    tracing::warn!(
                        run_id = %run_id,
                        node = %uid,
                        session_id = %session_id,
                        error = %e,
                        "reconcile: harness::status poll failed; leaving node Running"
                    )
                })
                .ok()
        }
    };
    let confirm = || tokio::time::sleep(std::time::Duration::from_millis(SETTLE_CONFIRM_MS));
    let outcomes = settle_outcomes(running, record.abort, confirm, poll).await;
    // Read after the confirm, so node durations include it.
    let now = deps.now_ms();
    let mut cancelled = Vec::new();

    for (uid, outcome) in outcomes {
        let outcome = match outcome {
            NodeOutcome::Done(res) => match submitted_result(deps, record, &uid).await {
                Some(value) => NodeOutcome::Done(value),
                None => match json_node_outcome(def, &uid, res) {
                    Ok(value) => NodeOutcome::Done(value),
                    Err(reason) => {
                        let reason = format!("{NOT_SUBMITTED}, and the final message does not settle it either: {reason}");
                        correct_json_node(deps, record, &uid, reason, timeout_ms).await
                    }
                },
            },
            // A turn that fails after its result was accepted (its closing line, say)
            // still delivered: settle with that result.
            NodeOutcome::Failed(err) => match submitted_result(deps, record, &uid).await {
                Some(value) => NodeOutcome::Done(value),
                None => NodeOutcome::Failed(err),
            },
            other => other,
        };
        match outcome {
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
                    cp.result_error = Some(stored_error(&err).to_string());
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
                    cp.result_error = Some("cancelled".into());
                }
                cancelled.push(uid);
            }
        }
    }

    if let Some(caller) = caller_to_check(def, record, &cancelled).map(str::to_string) {
        match deps
            .iii
            .call(iii_sdk::CallRequest {
                function_id: "harness::status".into(),
                payload: json!({ "session_id": caller }),
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await
        {
            Ok(resp) if resp["status"] == "cancelled" => record.abort = true,
            Ok(_) => {}
            Err(e) => tracing::warn!(
                run_id = %record.run_id,
                caller = %caller,
                error = %e,
                "reconcile: caller status poll failed; not aborting"
            ),
        }
    }

    Ok(())
}

/// Each polled node's outcome. A node whose first `poll` reply reads finished is
/// polled again after `confirm` and settles from that reply only; all such nodes
/// share one `confirm`. Under `abort` the run is cancelled whatever the nodes read,
/// so the first reply settles. A failed poll (`None`) leaves its node out.
async fn settle_outcomes<C, Cf, P, Pf>(
    running: Vec<(String, String)>,
    abort: bool,
    confirm: C,
    poll: P,
) -> Vec<(String, NodeOutcome)>
where
    C: FnOnce() -> Cf,
    Cf: std::future::Future<Output = ()>,
    P: Fn(&str, &str) -> Pf,
    Pf: std::future::Future<Output = Option<Value>>,
{
    let mut outcomes = Vec::new();
    let mut settling = Vec::new();
    for (uid, session_id) in running {
        let Some(resp) = poll(&uid, &session_id).await else {
            continue;
        };
        match status_outcome(&resp, abort) {
            NodeOutcome::StillRunning => {}
            outcome if abort => outcomes.push((uid, outcome)),
            _ => settling.push((uid, session_id)),
        }
    }
    if settling.is_empty() {
        return outcomes;
    }
    confirm().await;
    for (uid, session_id) in settling {
        if let Some(resp) = poll(&uid, &session_id).await {
            outcomes.push((uid, status_outcome(&resp, abort)));
        }
    }
    outcomes
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
    fn caller_checked_only_for_a_cancelled_tolerant_node() {
        let mut def: WorkflowDef = serde_json::from_value(json!({
            "version": 1,
            "nodes": {
                "a": {"agent": {"model": "m"}, "input": {"from": "run_input"}, "on_error": "continue"},
                "b": {"agent": {"model": "m"}, "input": {"from": "run_input"}}
            },
            "output": {"from": "node:b"}
        }))
        .expect("def");
        let mut record = WorkflowRunRecord {
            caller_session_id: Some("S".into()),
            ..serde_json::from_value(json!({
                "run_id": "r", "step": 0, "status": "awaiting_nodes", "def_ref": "r",
                "input": {}, "created_at": 0, "updated_at": 0
            }))
            .expect("record")
        };
        assert_eq!(caller_to_check(&def, &record, &["a".into()]), Some("S"));
        assert_eq!(caller_to_check(&def, &record, &["a#2".into()]), Some("S"));
        // A strict node already fails the run; nothing cancelled, nothing to check.
        assert_eq!(caller_to_check(&def, &record, &["b".into()]), None);
        assert_eq!(caller_to_check(&def, &record, &[]), None);
        record.caller_session_id = None;
        assert_eq!(caller_to_check(&def, &record, &["a".into()]), None);
        def.nodes.clear();
        record.caller_session_id = Some("S".into());
        assert_eq!(caller_to_check(&def, &record, &["a".into()]), None);
    }

    fn verdict_schema() -> Value {
        json!({"type": "object", "properties": {"verdict": {"type": "string"}}, "required": ["verdict"]})
    }

    #[test]
    fn json_node_result_parses_text_and_tolerates_one_fence() {
        let schema = verdict_schema();
        for text in [
            r#"{"verdict": "pass"}"#,
            "  {\"verdict\": \"pass\"}\n",
            "```json\n{\"verdict\": \"pass\"}\n```",
            "```\n{\"verdict\": \"pass\"}\n```",
        ] {
            assert_eq!(
                json_node_result(Some(&schema), json!(text)),
                Ok(json!({"verdict": "pass"})),
                "{text:?}"
            );
        }
        // A result already JSON (a turn under the harness's JSON contract) is checked as is.
        assert_eq!(
            json_node_result(Some(&schema), json!({"verdict": "pass"})),
            Ok(json!({"verdict": "pass"}))
        );
        // No schema: any JSON value.
        assert_eq!(json_node_result(None, json!("[1, 2]")), Ok(json!([1, 2])));
    }

    #[test]
    fn json_node_result_rejects_prose_and_schema_misses() {
        let schema = verdict_schema();
        let err = json_node_result(Some(&schema), json!("waiting on job_x")).unwrap_err();
        assert!(err.starts_with("the answer is not a JSON value"), "{err}");
        let err = json_node_result(Some(&schema), json!(r#"{"other": 1}"#)).unwrap_err();
        assert!(
            err.starts_with("the result does not match the node's output schema")
                && err.contains("verdict"),
            "{err}"
        );
        let err = json_node_result(Some(&schema), json!({"verdict": 3})).unwrap_err();
        assert!(err.contains("schema"), "{err}");
    }

    #[test]
    fn json_node_outcome_parses_json_nodes_only() {
        let def: WorkflowDef = serde_json::from_value(json!({
            "version": 1,
            "nodes": {
                "j": {"agent": {"model": "m", "output": {"type": "json", "schema": verdict_schema()}},
                      "input": {"from": "run_input"}},
                "t": {"agent": {"model": "m", "output": {"type": "text"}}, "input": {"from": "run_input"}},
                "p": {"agent": {"model": "m"}, "input": {"from": "run_input"}}
            },
            "output": {"from": ["node:j", "node:t", "node:p"]}
        }))
        .expect("def");
        assert_eq!(
            json_node_outcome(&def, "j#2", json!("{\"verdict\": \"ok\"}")),
            Ok(json!({"verdict": "ok"}))
        );
        let err = json_node_outcome(&def, "j", json!("done, see above")).unwrap_err();
        assert!(
            err.starts_with("the answer is not a JSON value")
                && err.ends_with(&format!("Required JSON Schema: {}", verdict_schema())),
            "{err}"
        );
        // Text nodes keep their text, even text that happens to be JSON.
        for uid in ["t", "p"] {
            assert_eq!(
                json_node_outcome(&def, uid, json!("{\"a\": 1}")),
                Ok(json!("{\"a\": 1}"))
            );
        }
    }

    #[test]
    fn a_null_schema_is_no_schema() {
        let output = json!({"type": "json", "schema": null});
        assert_eq!(output_schema(&output), None);
        assert_eq!(
            json_node_result(output_schema(&output), json!("[1]")),
            Ok(json!([1]))
        );
        assert_eq!(correction_reason(&output, "e".into()), "e");
    }

    #[test]
    fn stored_error_drops_the_schema_a_rejection_carries() {
        let output = json!({"type": "json", "schema": verdict_schema()});
        let reason = correction_reason(&output, "the answer is not a JSON value (eof)".into());
        assert!(reason.contains("\"required\""), "{reason}");
        // The harness's error after the hook's last deny is the deny reason.
        assert_eq!(
            stored_error(&reason),
            "the answer is not a JSON value (eof)"
        );
        // Reconcile's own rejection, once its corrections ran out.
        assert_eq!(
            stored_error(&format!("JSON node result rejected: {reason}")),
            "JSON node result rejected: the answer is not a JSON value (eof)"
        );
        // Any other error is stored whole.
        assert_eq!(
            stored_error("provider failed. Retry"),
            "provider failed. Retry"
        );
    }

    #[test]
    fn correction_payload_resends_the_retry_prompt_under_a_key_per_correction() {
        let payload = correction_payload("wf_r_a@r0", 1, "bad");
        assert_eq!(payload["session_id"], "wf_r_a@r0");
        assert_eq!(payload["idempotency_key"], "wf_r_a@r0#correction1");
        let message = payload["message"].as_str().unwrap();
        assert!(
            message.contains(": bad.") && !message.contains("{reason}"),
            "{message}"
        );
    }

    #[test]
    fn completed_modest_result_unaffected_by_cap() {
        let outcome = classify_terminal("completed", Some(json!({"k": "v"})), None);
        assert_eq!(outcome, NodeOutcome::Done(json!({"k": "v"})));
    }

    /// `settle_outcomes` over scripted `harness::status` replies, in order per uid
    /// (`None` is a failed poll). The outcomes, the uids polled in order, and how
    /// many times `confirm` ran; the confirm is counted, never slept.
    async fn settle(
        replies: Vec<(&str, Vec<Option<Value>>)>,
        abort: bool,
    ) -> (Vec<(String, NodeOutcome)>, Vec<String>, usize) {
        let running = replies
            .iter()
            .map(|(uid, _)| (uid.to_string(), format!("wf_{uid}")))
            .collect();
        let script: std::cell::RefCell<std::collections::HashMap<_, _>> = replies
            .into_iter()
            .map(|(uid, r)| (uid.to_string(), std::collections::VecDeque::from(r)))
            .collect::<std::collections::HashMap<_, _>>()
            .into();
        let polled = std::cell::RefCell::new(Vec::new());
        let confirms = std::cell::Cell::new(0);
        let poll = |uid: &str, session_id: &str| {
            assert_eq!(session_id, format!("wf_{uid}"));
            polled.borrow_mut().push(uid.to_string());
            let mut script = script.borrow_mut();
            let queue = script.get_mut(uid).expect("scripted node");
            std::future::ready(queue.pop_front().expect("one reply per poll"))
        };
        let confirm = || {
            confirms.set(confirms.get() + 1);
            std::future::ready(())
        };
        let outcomes = settle_outcomes(running, abort, confirm, poll).await;
        (outcomes, polled.into_inner(), confirms.get())
    }

    fn completed(result: Value) -> Option<Value> {
        Some(json!({"status": "completed", "result": result, "expects_wake": false}))
    }

    fn running() -> Option<Value> {
        Some(json!({"status": "running", "expects_wake": false}))
    }

    #[tokio::test]
    async fn wiring_ack_in_the_wake_gap_does_not_settle() {
        // The parking turn reads finished while its wake is claimed but not yet a turn
        // (or queued behind the completed write); by the confirm the woken turn runs.
        let parked = Some(json!({"status": "completed", "result": "ack", "expects_wake": true}));
        let (outcomes, polled, confirms) = settle(
            vec![
                ("a", vec![completed(json!("wired; waiting")), running()]),
                ("b", vec![completed(json!("wired; waiting")), parked]),
            ],
            false,
        )
        .await;
        assert_eq!(confirms, 1);
        assert_eq!(polled, ["a", "b", "a", "b"]);
        assert_eq!(
            outcomes,
            [
                ("a".to_string(), NodeOutcome::StillRunning),
                ("b".to_string(), NodeOutcome::StillRunning)
            ]
        );
    }

    #[tokio::test]
    async fn woken_turn_finished_within_the_delay_settles_its_own_result() {
        let (outcomes, _, confirms) = settle(
            vec![(
                "a",
                vec![
                    completed(json!("wired; waiting")),
                    completed(json!({"answer": 42})),
                ],
            )],
            false,
        )
        .await;
        assert_eq!(confirms, 1);
        assert_eq!(
            outcomes,
            [("a".to_string(), NodeOutcome::Done(json!({"answer": 42})))]
        );
    }

    #[tokio::test]
    async fn plain_finished_node_settles_after_the_confirming_poll() {
        let (outcomes, polled, confirms) = settle(
            vec![
                ("a", vec![completed(json!("x")), completed(json!("x"))]),
                ("b", vec![completed(json!("y")), completed(json!("y"))]),
            ],
            false,
        )
        .await;
        // Every node settling in one pass shares one confirm.
        assert_eq!(confirms, 1);
        assert_eq!(polled, ["a", "b", "a", "b"]);
        assert_eq!(
            outcomes,
            [
                ("a".to_string(), NodeOutcome::Done(json!("x"))),
                ("b".to_string(), NodeOutcome::Done(json!("y")))
            ]
        );
    }

    #[tokio::test]
    async fn running_and_parked_nodes_skip_the_confirming_poll() {
        let parked = Some(json!({"status": "completed", "result": "ack", "expects_wake": true}));
        let (outcomes, polled, confirms) = settle(
            vec![
                ("running", vec![running()]),
                ("parked", vec![parked]),
                ("unknown", vec![Some(Value::Null)]),
                ("unreachable", vec![None]),
            ],
            false,
        )
        .await;
        assert_eq!(confirms, 0);
        assert_eq!(polled, ["running", "parked", "unknown", "unreachable"]);
        assert!(outcomes.is_empty(), "{outcomes:?}");
    }

    #[tokio::test]
    async fn failed_confirming_poll_leaves_the_node_running() {
        let (outcomes, polled, confirms) =
            settle(vec![("a", vec![completed(json!("x")), None])], false).await;
        assert_eq!(confirms, 1);
        assert_eq!(polled, ["a", "a"]);
        // No outcome: reconcile_run leaves the checkpoint Running for the next tick.
        assert!(outcomes.is_empty(), "{outcomes:?}");
    }

    #[tokio::test]
    async fn cancel_settles_from_the_confirming_poll() {
        let cancelled = || Some(json!({"status": "cancelled"}));
        let failed = || Some(json!({"status": "failed", "result_error": "boom"}));
        let (outcomes, polled, confirms) = settle(
            vec![
                ("a", vec![cancelled(), cancelled()]),
                ("b", vec![failed(), failed()]),
                // A cancel read once and then a woken turn: the second reply rules.
                ("c", vec![cancelled(), running()]),
            ],
            false,
        )
        .await;
        assert_eq!(confirms, 1);
        assert_eq!(polled, ["a", "b", "c", "a", "b", "c"]);
        assert_eq!(
            outcomes,
            [
                ("a".to_string(), NodeOutcome::Cancelled),
                ("b".to_string(), NodeOutcome::Failed("boom".into())),
                ("c".to_string(), NodeOutcome::StillRunning)
            ]
        );
    }

    #[tokio::test]
    async fn abort_skips_the_confirming_poll() {
        // The run is cancelled whatever the nodes read: the first reply settles.
        let (outcomes, polled, confirms) = settle(
            vec![
                ("a", vec![Some(json!({"status": "cancelled"}))]),
                ("b", vec![completed(json!("wired; waiting"))]),
            ],
            true,
        )
        .await;
        assert_eq!(confirms, 0);
        assert_eq!(polled, ["a", "b"]);
        assert_eq!(outcomes, [("a".to_string(), NodeOutcome::Cancelled)]);
    }
}
