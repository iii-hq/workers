use serde_json::{Value, json};

use crate::{
    config::WorkerConfig,
    error::WorkflowError,
    ids::node_result_key,
    state,
    types::{NodeCheckpoint, NodeState, WorkflowDef, WorkflowRunRecord},
};

// ---------------------------------------------------------------------------
// NodeOutcome
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum NodeOutcome {
    StillRunning,
    Done(Value),
    Failed(String),
    /// A failed turn the harness marks `failure.retryable`: the provider failed
    /// temporarily, so a later send into the session may succeed.
    Transient(String),
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
    // A stored null reads back as "no result", so it could never settle the node.
    if value.is_null() {
        return Err("the answer is null: deliver the value itself".into());
    }
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
/// repeated: the system-prompt contract is fail-open, so the model may not have it. Pure.
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
/// attempt, read back from the node's result slot; `Ok(None)` when it delivered
/// nothing. A failed read is an `Err`: the node stays as it is and is read again
/// next tick, never settled from its final message in place of an accepted submit.
async fn submitted_result(
    deps: &crate::functions::Deps,
    record: &WorkflowRunRecord,
    uid: &str,
) -> Result<Option<Value>, WorkflowError> {
    if !record.nodes.get(uid).is_some_and(|cp| cp.submitted) {
        return Ok(None);
    }
    state::get_node_result(&deps.iii, &record.run_id, uid)
        .await
        .inspect_err(|e| {
            tracing::warn!(run_id = %record.run_id, node = %uid, error = %e, "reconcile: submitted result unreadable; retrying next tick")
        })
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
// Transient-failure retry of read-only nodes
// ---------------------------------------------------------------------------

/// Transient-failure continues a read-only node uid gets, over all its attempts.
const MAX_TRANSIENT_RETRIES: u32 = 1;

/// Wait before the continue. The harness's own resumes run back to back, so they
/// all fall in the same outage; provider rate-limit windows are per minute.
const TRANSIENT_RETRY_DELAY_MS: i64 = 60_000;

/// The default `sweep_expression` period. Only the schedule-time estimate of the
/// send assumes it; the send-time check holds whatever the sweep runs at.
const SWEEP_PERIOD_MS: i64 = 60_000;

/// What a continue must have left of the node's `timeout_ms` window when sent: it
/// reloads the whole transcript, so one the timeout stops at once is paid for nothing.
const MIN_CONTINUE_MS: i64 = 300_000;

/// What continues a read-only node's failed turn, in its own session.
const CONTINUE_PROMPT: &str = "The provider failed temporarily and your previous turn ended \
    before you delivered your result. Continue from where you stopped: everything above still \
    stands, so do not repeat calls that already returned. Then deliver the complete result as \
    your instructions say.";

/// The node's own `timeout_ms`, else the worker default. Pure.
fn node_timeout_ms(def: &WorkflowDef, cfg: &WorkerConfig, uid: &str) -> u64 {
    crate::timeout::node_limits(
        def,
        uid,
        cfg.default_pending_timeout_ms,
        cfg.max_node_retries,
    )
    .0
}

/// At least `MIN_CONTINUE_MS` of the node's window, `timeout_ms` from its
/// attempt's send, is left at `at`. Pure.
fn continue_fits(cp: &NodeCheckpoint, timeout_ms: u64, at: i64) -> bool {
    let deadline = cp
        .pending_at
        .unwrap_or(at)
        .saturating_add(i64::try_from(timeout_ms).unwrap_or(i64::MAX));
    deadline.saturating_sub(at) >= MIN_CONTINUE_MS
}

/// Schedule the continue of a node whose turn failed transiently: true when it
/// is read-only, the run is not stopping, its uid has not used its retry, and
/// the continue, sent by the first sweep after the wait, still fits in its
/// `timeout_ms` window (`continue_fits`). Pure.
fn schedule_transient_retry(
    def: &WorkflowDef,
    uid: &str,
    cp: &mut NodeCheckpoint,
    abort: bool,
    timeout_ms: u64,
    now: i64,
) -> bool {
    let read_only = def
        .nodes
        .get(uid.split('#').next().unwrap_or(uid))
        .is_some_and(|n| n.read_only);
    let sent_by = now + TRANSIENT_RETRY_DELAY_MS + SWEEP_PERIOD_MS;
    if abort
        || !read_only
        || cp.transient_retries >= MAX_TRANSIENT_RETRIES
        || !continue_fits(cp, timeout_ms, sent_by)
    {
        return false;
    }
    cp.transient_retries += 1;
    cp.retry_at = Some(now + TRANSIENT_RETRY_DELAY_MS);
    true
}

/// The outcome of a node `uid` whose turn failed transiently with `err` and
/// delivered nothing: still running when its continue is scheduled
/// (`schedule_transient_retry`, against the node's own `timeout_ms`), else failed.
/// Pure.
fn transient_outcome(
    def: &WorkflowDef,
    cfg: &WorkerConfig,
    record: &mut WorkflowRunRecord,
    uid: &str,
    err: String,
    now: i64,
) -> NodeOutcome {
    let timeout_ms = node_timeout_ms(def, cfg, uid);
    let abort = record.abort;
    let scheduled = record
        .nodes
        .get_mut(uid)
        .is_some_and(|cp| schedule_transient_retry(def, uid, cp, abort, timeout_ms, now));
    if scheduled {
        tracing::info!(run_id = %record.run_id, node = %uid, "reconcile: read-only node failed transiently; continuing it in a minute");
        NodeOutcome::StillRunning
    } else {
        NodeOutcome::Failed(err)
    }
}

/// The node waits for a continue that is due. Pure.
fn retry_due(cp: &NodeCheckpoint, now: i64) -> bool {
    cp.state == NodeState::Running && cp.retry_at.is_some_and(|at| at <= now)
}

/// Whether to send a due node's continue, by its session's `harness::status`
/// reply. A session that moved on (a person or another sender continued it)
/// stops waiting, so the normal poll settles it. One still on the transient
/// failure but past `continue_fits` (a slower sweep, a held run lock) fails
/// with that error. Pure.
fn still_due(cp: &mut NodeCheckpoint, resp: &Value, timeout_ms: u64, now: i64) -> bool {
    let NodeOutcome::Transient(err) = status_outcome(resp, false, cp.submitted) else {
        cp.retry_at = None;
        return false;
    };
    if continue_fits(cp, timeout_ms, now) {
        return true;
    }
    cp.retry_at = None;
    cp.state = NodeState::Failed;
    cp.result_error = Some(stored_error(&err).to_string());
    false
}

/// The `harness::send` payload that continues a node's session. It names no
/// options, so the turn inherits the session's model, profile, functions and
/// folder; the key makes a resend after a lost reply a harness no-op. Pure.
fn continue_payload(session_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "idempotency_key": format!("{session_id}#transient-retry"),
        "message": CONTINUE_PROMPT,
    })
}

/// A continue's `harness::send` reply (`Ok` with the new turn id) applied to the
/// node: sent, it is no longer waiting and keeps its attempt's `pending_at`, so
/// the continued turn runs in what is left of `timeout_ms`; refused outright, it
/// fails; any other error leaves it due for the next pass. Pure.
fn apply_continue_reply(cp: &mut NodeCheckpoint, reply: Result<Option<String>, String>) {
    match reply {
        Ok(turn_id) => {
            cp.retry_at = None;
            cp.turn_id = turn_id;
        }
        Err(e) if crate::timeout::is_permanent_send_error(&e) => {
            cp.state = NodeState::Failed;
            cp.result_error = Some(format!("transient retry refused: {e}"));
        }
        Err(_) => {}
    }
}

/// `harness::status` of the caller's session reads `cancelled`.
async fn caller_cancelled(
    deps: &crate::functions::Deps,
    caller: &str,
    timeout_ms: u64,
) -> Result<bool, iii_sdk::Error> {
    let resp = deps
        .iii
        .call(iii_sdk::CallRequest {
            function_id: "harness::status".into(),
            payload: json!({ "session_id": caller }),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await?;
    Ok(resp["status"] == "cancelled")
}

/// Send the continue of every node whose retry is due, on a run not stopping.
/// Each node's session is read first (`still_due`). A failed read or send leaves
/// the node due; a later pass fails it once too little of its window is left.
async fn send_due_retries(
    deps: &crate::functions::Deps,
    def: &WorkflowDef,
    cfg: &WorkerConfig,
    record: &mut WorkflowRunRecord,
    now: i64,
) {
    if record.abort {
        return;
    }
    let due: Vec<(String, String)> = record
        .nodes
        .iter()
        .filter(|(_, cp)| retry_due(cp, now))
        .filter_map(|(uid, cp)| cp.session_id.clone().map(|sid| (uid.clone(), sid)))
        .collect();
    let run_id = record.run_id.clone();
    let call = |function_id: &str, payload: Value| {
        deps.iii.call(iii_sdk::CallRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(cfg.dispatch_timeout_ms),
        })
    };
    for (uid, sid) in due {
        let status = call(
            "harness::status",
            json!({ "session_id": sid, "verbose": true }),
        )
        .await;
        let timeout_ms = node_timeout_ms(def, cfg, &uid);
        let Some(cp) = record.nodes.get_mut(&uid) else {
            continue;
        };
        // The reads before this one took time: check the window as of now.
        match status {
            Ok(resp) if still_due(cp, &resp, timeout_ms, deps.now_ms()) => {}
            Ok(_) => continue,
            Err(e) => {
                tracing::warn!(run_id = %run_id, node = %uid, error = %e, "reconcile: status read before a transient retry failed; retrying next pass");
                continue;
            }
        }
        let reply = call("harness::send", continue_payload(&sid))
            .await
            .map(|resp| resp["turn_id"].as_str().map(str::to_string))
            .map_err(|e| e.to_string());
        match &reply {
            Ok(_) => {
                tracing::info!(run_id = %run_id, node = %uid, "reconcile: continued a read-only node after a transient provider failure")
            }
            Err(e) => {
                tracing::warn!(run_id = %run_id, node = %uid, error = %e, "reconcile: transient retry send failed")
            }
        }
        apply_continue_reply(cp, reply);
    }
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

/// The reply's session holds an armed wake, or is about to be reseeded. Pure.
fn wake_armed(resp: &Value) -> bool {
    resp["expects_wake"].as_bool().unwrap_or(false)
}

/// The outcome a node's `harness::status` reply gives it on a run stopped or not
/// (`abort`), for an attempt that has `submitted` its result or not. Pure.
pub(crate) fn status_outcome(resp: &Value, abort: bool, submitted: bool) -> NodeOutcome {
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
    // An attempt that submitted has delivered: its wake is a leftover, so the
    // turn's own status settles it, and reconcile then stops its session.
    if resp.is_null() || (!submitted && wake_armed(resp)) {
        return NodeOutcome::StillRunning;
    }
    let status = resp["status"].as_str().unwrap_or("running");
    let result_error = resp["result_error"].as_str().map(str::to_string);
    match classify_terminal(status, resp.get("result").cloned(), result_error) {
        // A stopped parked node reads its wiring ack until the harness records the cancel.
        NodeOutcome::Done(_) if abort => NodeOutcome::StillRunning,
        // An older harness reports no `failure`: the turn fails the node.
        NodeOutcome::Failed(err) if status == "failed" && resp["failure"]["retryable"] == true => {
            NodeOutcome::Transient(err)
        }
        outcome => outcome,
    }
}

// ---------------------------------------------------------------------------
// reconcile_run
// ---------------------------------------------------------------------------

/// The caller session to ask `harness::status` on a run not already stopping,
/// when this pass cancelled an `on_error: continue` node or a node waits for its
/// transient continue. Stopping the caller cancels every in-flight node in its
/// tree: a tolerant node would otherwise settle as null and let its consumers
/// fire, and the stop skips a waiting node, whose turn already ended, so every
/// pass checks while one waits. Pure.
fn caller_to_check<'a>(
    def: &WorkflowDef,
    record: &'a WorkflowRunRecord,
    cancelled: &[String],
) -> Option<&'a str> {
    let tolerant = cancelled
        .iter()
        .any(|uid| crate::dag::tolerant(def, uid.split('#').next().unwrap_or(uid)));
    let waiting = record
        .nodes
        .values()
        .any(|cp| cp.state == NodeState::Running && cp.retry_at.is_some());
    record
        .caller_session_id
        .as_deref()
        .filter(|_| (tolerant || waiting) && !record.abort)
}

/// The `(uid, session_id, submitted)` of every node to poll: Running with a
/// session, and not waiting for a transient continue (`send_due_retries` reads
/// that one when it is due). Pure.
fn pollable(record: &WorkflowRunRecord) -> Vec<(String, String, bool)> {
    record
        .nodes
        .iter()
        .filter(|(_, cp)| cp.state == NodeState::Running && cp.retry_at.is_none())
        .filter_map(|(uid, cp)| {
            cp.session_id
                .as_ref()
                .map(|sid| (uid.clone(), sid.clone(), cp.submitted))
        })
        .collect()
}

/// For each `Running` checkpoint that has a `session_id`, poll
/// `harness::status` and advance the checkpoint to its terminal state (Done /
/// Failed / Cancelled).  Skips unknown sessions (null response) and sessions
/// still awaiting a wake (`expects_wake` — the run's outcome arrives on a
/// later turn in the same session). A finished reply is trusted only from the
/// confirming poll `SETTLE_CONFIRM_MS` later (`settle_outcomes`). An attempt that
/// submitted its result settles despite an armed wake; a node settled that way
/// has the run written, then its session stopped so the wake never runs. A
/// tolerant node cancelled by a stop of the caller's turn aborts the run
/// (`caller_to_check`); a failed caller poll defers that cancel to the next pass.
/// A read-only node whose turn failed transiently waits for its continue
/// (`transient_outcome`) and is not polled (`pollable`) until `send_due_retries`
/// sends it; while it waits, a stopped caller turn aborts the run, and a failed
/// caller poll holds every continue for the next pass.
pub async fn reconcile_run(
    deps: &crate::functions::Deps,
    def: &WorkflowDef,
    record: &mut WorkflowRunRecord,
) -> Result<(), WorkflowError> {
    // Owned, so no &mut reference is held across .await points.
    let running = pollable(record);

    let cfg = deps.cfg().await;
    let timeout_ms = cfg.dispatch_timeout_ms;
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
    // Sessions of nodes settled while a wake was still armed.
    let mut leftover = Vec::new();

    for (uid, outcome, armed) in outcomes {
        let outcome = match outcome {
            NodeOutcome::Done(res) => match submitted_result(deps, record, &uid).await {
                Err(_) => NodeOutcome::StillRunning,
                Ok(Some(value)) => NodeOutcome::Done(value),
                Ok(None) => match json_node_outcome(def, &uid, res) {
                    Ok(value) => NodeOutcome::Done(value),
                    Err(reason) => {
                        let reason = format!(
                            "{NOT_SUBMITTED}, and the final message does not settle it either: {reason}"
                        );
                        correct_json_node(deps, record, &uid, reason, timeout_ms).await
                    }
                },
            },
            // A turn that fails after its result was accepted (its closing line, say)
            // still delivered: settle with that result.
            outcome @ (NodeOutcome::Failed(_) | NodeOutcome::Transient(_)) => {
                match submitted_result(deps, record, &uid).await {
                    Err(_) => NodeOutcome::StillRunning,
                    Ok(Some(value)) => NodeOutcome::Done(value),
                    Ok(None) => match outcome {
                        NodeOutcome::Transient(err) => {
                            transient_outcome(def, &cfg, record, &uid, err, now)
                        }
                        failed => failed,
                    },
                }
            }
            other => other,
        };
        // Left armed, the wake would run a turn in the session of a settled node.
        if armed && outcome != NodeOutcome::StillRunning {
            leftover.extend(record.nodes.get(&uid).and_then(|cp| cp.session_id.clone()));
        }
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
            NodeOutcome::Failed(err) | NodeOutcome::Transient(err) => {
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

    let mut caller_unread = false;
    if let Some(caller) = caller_to_check(def, record, &cancelled).map(str::to_string) {
        match caller_cancelled(deps, &caller, timeout_ms).await {
            Ok(true) => record.abort = true,
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(
                    run_id = %record.run_id,
                    caller = %caller,
                    error = %e,
                    "reconcile: caller status poll failed; re-checking next pass"
                );
                defer_tolerant_cancels(def, record, &cancelled);
                caller_unread = true;
            }
        }
    }

    // The caller may have stopped: no continue goes out until a pass reads it.
    if !caller_unread {
        send_due_retries(deps, def, &cfg, record, now).await;
    }

    if !leftover.is_empty() {
        // Written first: the stopped session ends cancelled, which would cancel a
        // node still stored as Running if this pass's write were lost.
        crate::functions::tick::stamp_finished(&mut record.nodes, now);
        record.updated_at = record.updated_at.max(now);
        state::put_run(&deps.iii, record).await?;
        for sid in &leftover {
            crate::functions::harness_stop_session(deps, sid, timeout_ms).await;
        }
    }

    Ok(())
}

/// Put the tolerant nodes this pass cancelled back to Running when the caller
/// check could not run: settled, they would let their consumers fire, and no
/// later pass would check the caller again. The next poll cancels them again and
/// retries the check; the node's timeout is the backstop. Pure.
fn defer_tolerant_cancels(def: &WorkflowDef, record: &mut WorkflowRunRecord, cancelled: &[String]) {
    for uid in cancelled {
        if !crate::dag::tolerant(def, uid.split('#').next().unwrap_or(uid)) {
            continue;
        }
        if let Some(cp) = record.nodes.get_mut(uid) {
            cp.state = NodeState::Running;
            cp.result_error = None;
        }
    }
}

/// Each polled node's outcome, from `running` (uid, session_id, submitted), and
/// whether the reply it settled from had a wake armed. A node whose first `poll`
/// reply reads finished is polled again after `confirm` and settles from that
/// reply only; all such nodes share one `confirm`. Under `abort` the run is
/// cancelled whatever the nodes read, so the first reply settles. A failed poll
/// (`None`) leaves its node out.
async fn settle_outcomes<C, Cf, P, Pf>(
    running: Vec<(String, String, bool)>,
    abort: bool,
    confirm: C,
    poll: P,
) -> Vec<(String, NodeOutcome, bool)>
where
    C: FnOnce() -> Cf,
    Cf: std::future::Future<Output = ()>,
    P: Fn(&str, &str) -> Pf,
    Pf: std::future::Future<Output = Option<Value>>,
{
    let mut outcomes = Vec::new();
    let mut settling = Vec::new();
    for (uid, session_id, submitted) in running {
        let Some(resp) = poll(&uid, &session_id).await else {
            continue;
        };
        match status_outcome(&resp, abort, submitted) {
            NodeOutcome::StillRunning => {}
            outcome if abort => outcomes.push((uid, outcome, wake_armed(&resp))),
            _ => settling.push((uid, session_id, submitted)),
        }
    }
    if settling.is_empty() {
        return outcomes;
    }
    confirm().await;
    for (uid, session_id, submitted) in settling {
        if let Some(resp) = poll(&uid, &session_id).await {
            outcomes.push((
                uid,
                status_outcome(&resp, abort, submitted),
                wake_armed(&resp),
            ));
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
        // A run already stopping needs no check.
        record.caller_session_id = Some("S".into());
        record.abort = true;
        assert_eq!(caller_to_check(&def, &record, &["a".into()]), None);
        record.abort = false;

        // A failed caller poll puts the tolerant cancel back; the strict one stays.
        for uid in ["a", "b"] {
            record.nodes.insert(
                uid.into(),
                crate::types::NodeCheckpoint {
                    state: NodeState::Cancelled,
                    result_error: Some("cancelled".into()),
                    ..Default::default()
                },
            );
        }
        defer_tolerant_cancels(&def, &mut record, &["a".into(), "b".into()]);
        assert_eq!(record.nodes["a"].state, NodeState::Running);
        assert_eq!(record.nodes["a"].result_error, None);
        assert_eq!(record.nodes["b"].state, NodeState::Cancelled);

        def.nodes.clear();
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
    fn a_null_answer_never_settles_a_json_node() {
        for (schema, result) in [
            (None, json!("null")),
            (None, Value::Null),
            (Some(json!({})), json!("null")),
        ] {
            let err = json_node_result(schema.as_ref(), result).unwrap_err();
            assert!(err.contains("null"), "{err}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreadable_submit_is_an_error_not_a_missing_result() {
        let deps = crate::functions::Deps {
            iii: std::sync::Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:9")),
            cfg: Default::default(),
            locks: Default::default(),
            events: std::sync::Arc::new(crate::events::test_support::RecordingDeliverer::new(
                false,
            )),
        };
        let mut record: WorkflowRunRecord = serde_json::from_value(json!({
            "run_id": "r", "step": 0, "status": "awaiting_nodes", "def_ref": "r",
            "input": {}, "created_at": 0, "updated_at": 0
        }))
        .expect("record");
        record.nodes.insert(
            "j".into(),
            crate::types::NodeCheckpoint {
                submitted: true,
                ..Default::default()
            },
        );
        // The client never connects: the slot read times out.
        assert!(submitted_result(&deps, &record, "j").await.is_err());
        record.nodes.get_mut("j").unwrap().submitted = false;
        assert!(matches!(
            submitted_result(&deps, &record, "j").await,
            Ok(None)
        ));
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

    /// `settle_outcomes` over scripted `harness::status` replies, in order per
    /// (uid, submitted) (`None` is a failed poll). The outcomes, the uids polled in
    /// order, and how many times `confirm` ran; the confirm is counted, never slept.
    async fn settle_nodes(
        replies: Vec<(&str, bool, Vec<Option<Value>>)>,
        abort: bool,
    ) -> (Vec<(String, NodeOutcome, bool)>, Vec<String>, usize) {
        let running = replies
            .iter()
            .map(|(uid, submitted, _)| (uid.to_string(), format!("wf_{uid}"), *submitted))
            .collect();
        let script: std::cell::RefCell<std::collections::HashMap<_, _>> = replies
            .into_iter()
            .map(|(uid, _, r)| (uid.to_string(), std::collections::VecDeque::from(r)))
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

    /// [`settle_nodes`] for nodes that submitted nothing, without the wake flag.
    async fn settle(
        replies: Vec<(&str, Vec<Option<Value>>)>,
        abort: bool,
    ) -> (Vec<(String, NodeOutcome)>, Vec<String>, usize) {
        let replies = replies
            .into_iter()
            .map(|(uid, r)| (uid, false, r))
            .collect();
        let (outcomes, polled, confirms) = settle_nodes(replies, abort).await;
        let outcomes = outcomes
            .into_iter()
            .map(|(uid, outcome, _)| (uid, outcome))
            .collect();
        (outcomes, polled, confirms)
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

    fn parked() -> Value {
        json!({"status": "completed", "result": "ack", "expects_wake": true})
    }

    #[test]
    fn submitted_reply_ignores_the_armed_wake() {
        // Not submitted: the wiring ack of a one-way agent keeps waiting.
        assert_eq!(
            status_outcome(&parked(), false, false),
            NodeOutcome::StillRunning
        );
        assert_eq!(
            status_outcome(&parked(), false, true),
            NodeOutcome::Done(json!("ack"))
        );
        // A stopped run still waits for the harness to record the cancel.
        assert_eq!(
            status_outcome(&parked(), true, true),
            NodeOutcome::StillRunning
        );
        let busy = json!({"status": "running", "expects_wake": true});
        assert_eq!(
            status_outcome(&busy, false, true),
            NodeOutcome::StillRunning
        );
        let failed = json!({"status": "failed", "result_error": "boom", "expects_wake": true});
        assert_eq!(
            status_outcome(&failed, false, true),
            NodeOutcome::Failed("boom".into())
        );
        assert_eq!(
            status_outcome(&Value::Null, false, true),
            NodeOutcome::StillRunning
        );
    }

    #[tokio::test]
    async fn submitted_node_settles_with_a_wake_still_armed() {
        let failed_armed =
            || Some(json!({"status": "failed", "result_error": "boom", "expects_wake": true}));
        let cancelled_armed = || Some(json!({"status": "cancelled", "expects_wake": true}));
        let (outcomes, polled, confirms) = settle_nodes(
            vec![
                ("sub", true, vec![Some(parked()), Some(parked())]),
                ("ack", false, vec![Some(parked())]),
                (
                    "plain",
                    false,
                    vec![completed(json!("x")), completed(json!("x"))],
                ),
                ("fail", true, vec![failed_armed(), failed_armed()]),
                ("cancel", true, vec![cancelled_armed(), cancelled_armed()]),
            ],
            false,
        )
        .await;
        assert_eq!(confirms, 1);
        assert_eq!(
            polled,
            [
                "sub", "ack", "plain", "fail", "cancel", "sub", "plain", "fail", "cancel"
            ]
        );
        assert_eq!(
            outcomes,
            [
                ("sub".to_string(), NodeOutcome::Done(json!("ack")), true),
                ("plain".to_string(), NodeOutcome::Done(json!("x")), false),
                ("fail".to_string(), NodeOutcome::Failed("boom".into()), true),
                ("cancel".to_string(), NodeOutcome::Cancelled, true)
            ]
        );
    }

    fn transient() -> Value {
        json!({"status": "failed", "result_error": "The provider temporarily failed.",
               "failure": {"code": "router/provider_transient", "retryable": true},
               "expects_wake": false})
    }

    #[test]
    fn a_retryable_failure_is_transient_only_when_the_harness_says_so() {
        let err = || "The provider temporarily failed.".to_string();
        assert_eq!(
            status_outcome(&transient(), false, false),
            NodeOutcome::Transient(err())
        );
        // An older harness reports no `failure`.
        let mut older = transient();
        older.as_object_mut().unwrap().remove("failure");
        assert_eq!(
            status_outcome(&older, false, false),
            NodeOutcome::Failed(err())
        );
        let mut permanent = transient();
        permanent["failure"]["retryable"] = json!(false);
        assert_eq!(
            status_outcome(&permanent, false, false),
            NodeOutcome::Failed(err())
        );
        // Only a failed turn: a completed one with an error is the node's own failure.
        let mut completed = transient();
        completed["status"] = json!("completed");
        assert_eq!(
            status_outcome(&completed, false, false),
            NodeOutcome::Failed(err())
        );
        // A parked node keeps waiting on its wake.
        let mut armed = transient();
        armed["expects_wake"] = json!(true);
        assert_eq!(
            status_outcome(&armed, false, false),
            NodeOutcome::StillRunning
        );
    }

    #[tokio::test]
    async fn a_transient_failure_settles_only_from_the_confirming_poll() {
        let (outcomes, polled, confirms) = settle(
            vec![
                ("a", vec![Some(transient()), Some(transient())]),
                ("b", vec![Some(transient()), running()]),
            ],
            false,
        )
        .await;
        assert_eq!(confirms, 1);
        assert_eq!(polled, ["a", "b", "a", "b"]);
        assert_eq!(
            outcomes,
            [
                (
                    "a".to_string(),
                    NodeOutcome::Transient("The provider temporarily failed.".into())
                ),
                ("b".to_string(), NodeOutcome::StillRunning)
            ]
        );
    }

    /// `ro` read-only, `rw` not; `ro` fans out, and a loop pass of it reads as `ro#it1`.
    fn retry_def() -> WorkflowDef {
        serde_json::from_value(json!({
            "nodes": {
                "ro": {"agent": {"model": "m"}, "input": {"from": "fanout_item"},
                       "fanout": {"items": [1, 2, 3]}, "read_only": true},
                "rw": {"agent": {"model": "m"}, "input": {"from": "run_input"}}
            },
            "output": {"from": ["node:ro", "node:rw"]}
        }))
        .expect("def")
    }

    fn bare_record() -> WorkflowRunRecord {
        serde_json::from_value(json!({
            "run_id": "r", "step": 0, "status": "awaiting_nodes", "def_ref": "r",
            "input": {}, "created_at": 0, "updated_at": 0
        }))
        .expect("record")
    }

    fn running_since(pending_at: i64) -> crate::types::NodeCheckpoint {
        crate::types::NodeCheckpoint {
            state: NodeState::Running,
            session_id: Some("wf_r_ro@r0".into()),
            turn_id: Some("t1".into()),
            pending_at: Some(pending_at),
            ..Default::default()
        }
    }

    #[test]
    fn only_a_read_only_node_with_its_retry_and_time_left_is_scheduled() {
        let def = retry_def();
        let timeout = 600_000;
        let now = 1_000;
        for uid in ["ro", "ro#2", "ro#it1"] {
            let mut cp = running_since(0);
            assert!(
                schedule_transient_retry(&def, uid, &mut cp, false, timeout, now),
                "{uid}"
            );
            assert_eq!(cp.transient_retries, 1);
            assert_eq!(cp.retry_at, Some(now + TRANSIENT_RETRY_DELAY_MS));
            // The budget is spent: a second failure fails the node, untouched.
            let spent = cp.clone();
            assert!(!schedule_transient_retry(
                &def,
                uid,
                &mut cp,
                false,
                timeout,
                now + 90_000
            ));
            assert_eq!(cp, spent);
        }
        // Not read-only, a node the definition lacks, a stopping run: never.
        for (uid, abort) in [("rw", false), ("gone", false), ("ro", true)] {
            let mut cp = running_since(0);
            assert!(
                !schedule_transient_retry(&def, uid, &mut cp, abort, timeout, now),
                "{uid}"
            );
            assert_eq!(cp, running_since(0));
        }
    }

    #[test]
    fn a_retry_without_time_to_work_is_not_scheduled() {
        let def = retry_def();
        // The wait, one sweep period and the continue's own minimum must fit in
        // what is left of the window.
        let needed = TRANSIENT_RETRY_DELAY_MS + SWEEP_PERIOD_MS + MIN_CONTINUE_MS;
        let timeout = 1_200_000;
        let mut cp = running_since(0);
        assert!(!schedule_transient_retry(
            &def,
            "ro",
            &mut cp,
            false,
            timeout,
            1_200_000 - needed + 1
        ));
        assert_eq!(cp, running_since(0));
        assert!(schedule_transient_retry(
            &def,
            "ro",
            &mut cp,
            false,
            timeout,
            1_200_000 - needed
        ));
        // A 40-minute node (timeout_ms 42 min) whose harness resumes give up at
        // minute 40 fails at once: its continue would be stopped as it starts.
        let mut cp = running_since(0);
        assert!(!schedule_transient_retry(
            &def, "ro", &mut cp, false, 2_520_000, 2_400_000
        ));
        assert_eq!(cp, running_since(0));
    }

    #[test]
    fn a_transient_failure_keeps_a_read_only_node_running_under_its_own_timeout() {
        let mut def = retry_def();
        def.nodes.get_mut("ro").unwrap().timeout_ms = Some(2_520_000);
        // Neither the worker default nor the dispatch timeout leaves room.
        let cfg = WorkerConfig {
            default_pending_timeout_ms: 1_000,
            dispatch_timeout_ms: 30_000,
            ..Default::default()
        };
        let mut record = bare_record();
        record.nodes.insert("ro#2".into(), running_since(0));
        record.nodes.insert("rw".into(), running_since(0));
        let err = || "The provider temporarily failed.".to_string();
        let now = 60_000;

        assert_eq!(
            transient_outcome(&def, &cfg, &mut record, "ro#2", err(), now),
            NodeOutcome::StillRunning
        );
        assert_eq!(
            record.nodes["ro#2"].retry_at,
            Some(now + TRANSIENT_RETRY_DELAY_MS)
        );
        // The second transient failure fails it.
        assert_eq!(
            transient_outcome(&def, &cfg, &mut record, "ro#2", err(), now + 200_000),
            NodeOutcome::Failed(err())
        );
        // Not read-only: fails. A stopping run: fails.
        assert_eq!(
            transient_outcome(&def, &cfg, &mut record, "rw", err(), now),
            NodeOutcome::Failed(err())
        );
        record.nodes.insert("ro#1".into(), running_since(0));
        record.abort = true;
        assert_eq!(
            transient_outcome(&def, &cfg, &mut record, "ro#1", err(), now),
            NodeOutcome::Failed(err())
        );
        assert_eq!(record.nodes["ro#1"], running_since(0));
    }

    #[test]
    fn a_node_waiting_for_its_continue_is_not_polled() {
        let mut record = bare_record();
        let sid = |uid: &str| Some(format!("wf_r_{uid}@r0"));
        for (uid, cp) in [
            (
                "a",
                NodeCheckpoint {
                    session_id: sid("a"),
                    ..running_since(0)
                },
            ),
            (
                "b",
                NodeCheckpoint {
                    session_id: sid("b"),
                    retry_at: Some(5_000),
                    ..running_since(0)
                },
            ),
            (
                "c",
                NodeCheckpoint {
                    session_id: sid("c"),
                    submitted: true,
                    ..running_since(0)
                },
            ),
            (
                "d",
                NodeCheckpoint {
                    session_id: None,
                    ..running_since(0)
                },
            ),
            (
                "e",
                NodeCheckpoint {
                    session_id: sid("e"),
                    state: NodeState::Done,
                    ..running_since(0)
                },
            ),
        ] {
            record.nodes.insert(uid.into(), cp);
        }
        assert_eq!(
            pollable(&record),
            [
                ("a".to_string(), "wf_r_a@r0".to_string(), false),
                ("c".to_string(), "wf_r_c@r0".to_string(), true)
            ]
        );
    }

    #[test]
    fn caller_checked_on_every_pass_while_a_node_waits() {
        let def = retry_def();
        let mut record = WorkflowRunRecord {
            caller_session_id: Some("S".into()),
            ..bare_record()
        };
        record.nodes.insert(
            "ro".into(),
            NodeCheckpoint {
                retry_at: Some(5_000),
                ..running_since(0)
            },
        );
        assert_eq!(caller_to_check(&def, &record, &[]), Some("S"));
        record.abort = true;
        assert_eq!(caller_to_check(&def, &record, &[]), None);
        record.abort = false;
        // Continued: its turn is in flight again, so a caller stop cancels it.
        record.nodes.get_mut("ro").unwrap().retry_at = None;
        assert_eq!(caller_to_check(&def, &record, &[]), None);
    }

    #[test]
    fn a_retry_is_due_from_its_time_on_a_running_node() {
        let mut cp = running_since(0);
        assert!(!retry_due(&cp, 5_000), "no retry scheduled");
        cp.retry_at = Some(5_000);
        assert!(!retry_due(&cp, 4_999));
        assert!(retry_due(&cp, 5_000));
        assert!(retry_due(&cp, 9_000));
        cp.state = NodeState::Cancelled;
        assert!(!retry_due(&cp, 9_000));
    }

    #[test]
    fn a_session_that_moved_on_stops_waiting_for_its_retry() {
        let waiting = || crate::types::NodeCheckpoint {
            retry_at: Some(5_000),
            ..running_since(0)
        };
        let mut cp = waiting();
        assert!(still_due(&mut cp, &transient(), 600_000, 5_000));
        assert_eq!(cp.retry_at, Some(5_000));
        // Someone continued the session, it already finished, or it is gone:
        // the normal poll settles the node from here.
        for resp in [
            running().unwrap(),
            completed(json!("done")).unwrap(),
            Value::Null,
        ] {
            let mut cp = waiting();
            assert!(!still_due(&mut cp, &resp, 600_000, 5_000), "{resp}");
            assert_eq!(cp.retry_at, None);
            assert!(!retry_due(&cp, 9_000));
            assert_eq!(cp.state, NodeState::Running);
        }
    }

    #[test]
    fn a_late_pass_fails_the_node_instead_of_sending_its_continue() {
        let waiting = || crate::types::NodeCheckpoint {
            transient_retries: 1,
            retry_at: Some(5_000),
            ..running_since(0)
        };
        let last = 600_000 - MIN_CONTINUE_MS;
        let mut cp = waiting();
        assert!(still_due(&mut cp, &transient(), 600_000, last));
        assert_eq!(cp, waiting());
        // A slower sweep reached it later: the continue would have no time to work.
        let mut cp = waiting();
        assert!(!still_due(&mut cp, &transient(), 600_000, last + 1));
        assert_eq!(cp.state, NodeState::Failed);
        assert_eq!(
            cp.result_error.as_deref(),
            Some("The provider temporarily failed.")
        );
        assert_eq!(cp.retry_at, None);
    }

    #[test]
    fn continue_payload_names_no_options_under_one_key_per_session() {
        let payload = continue_payload("wf_r_ro#2@r0");
        assert_eq!(
            payload,
            json!({
                "session_id": "wf_r_ro#2@r0",
                "idempotency_key": "wf_r_ro#2@r0#transient-retry",
                "message": CONTINUE_PROMPT
            })
        );
    }

    #[test]
    fn a_continue_reply_clears_the_wait_fails_the_node_or_leaves_it_due() {
        let waiting = || crate::types::NodeCheckpoint {
            transient_retries: 1,
            retry_at: Some(5_000),
            ..running_since(7)
        };
        let mut cp = waiting();
        apply_continue_reply(&mut cp, Ok(Some("t2".into())));
        assert_eq!(cp.retry_at, None);
        assert_eq!(cp.turn_id.as_deref(), Some("t2"));
        // The continued turn runs in what is left of the attempt's window.
        assert_eq!(cp.pending_at, Some(7));
        assert_eq!(cp.state, NodeState::Running);

        let mut cp = waiting();
        let refused = "remote error (invocation_failed): harness/invalid_request: bad";
        apply_continue_reply(&mut cp, Err(refused.into()));
        assert_eq!(cp.state, NodeState::Failed);
        assert_eq!(
            cp.result_error.as_deref(),
            Some(&*format!("transient retry refused: {refused}"))
        );

        let mut cp = waiting();
        apply_continue_reply(&mut cp, Err("invocation timed out".into()));
        assert_eq!(cp, waiting());
        assert!(retry_due(&cp, 5_000));
    }

    #[tokio::test]
    async fn submitted_node_still_working_or_woken_stays_running() {
        let (outcomes, polled, confirms) = settle_nodes(
            vec![
                (
                    "busy",
                    true,
                    vec![Some(json!({"status": "running", "expects_wake": true}))],
                ),
                ("woken", true, vec![Some(parked()), running()]),
            ],
            false,
        )
        .await;
        assert_eq!(confirms, 1);
        assert_eq!(polled, ["busy", "woken", "woken"]);
        assert_eq!(
            outcomes,
            [("woken".to_string(), NodeOutcome::StillRunning, false)]
        );
    }
}
