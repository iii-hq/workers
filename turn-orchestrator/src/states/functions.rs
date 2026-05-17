//! `function_prepare`, `function_execute`, `function_finalize` handlers.

use harness_types::{
    AgentEvent, AgentMessage, AssistantMessage, ContentBlock, FunctionCall, FunctionResult,
    FunctionResultMessage, TextContent,
};
use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;

use crate::agent_call::TOOL_NAME as AGENT_CALL_TOOL_NAME;
use crate::events;
use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};

const TOPIC_BEFORE: &str = "agent::before_function_call";
const TOPIC_AFTER: &str = "agent::after_function_call";
const HOOK_TIMEOUT_MS: u64 = 10_000;

/// Build the prefilled FunctionResult for a hook reply that returned
/// `block: true`. Pending replies produce a structured tool-result-style
/// payload the LLM can recognise; non-pending blocked replies stay as a
/// plain text reason.
///
/// Two shapes terminate the turn:
///   - `status: "pending"` — real result comes asynchronously via
///     `approval::consume`; continuing would loop the model with a
///     stale pending notice.
///   - `denial.kind: "state_error"` — the hook bus or a subscriber's
///     state-write failed (finding #1 fail-closed path). Continuing
///     would let the model retry against an unhealthy substrate and
///     potentially bypass the gate; we stop cleanly so the operator
///     can investigate.
pub(crate) fn prefilled_result_for_block(
    merged: &Value,
    call_id: &str,
    function_id: &str,
) -> FunctionResult {
    if merged.get("status").and_then(Value::as_str) == Some("pending") {
        let body = json!({
            "status": "pending_approval",
            "call_id": call_id,
            "function_id": function_id,
            "message": "Awaiting human approval. The result will be reported in a future turn."
        });
        return FunctionResult {
            content: vec![ContentBlock::Text(TextContent {
                text: serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string()),
            })],
            details: json!({ "pending_approval": true, "call_id": call_id }),
            // Stop the loop. Without this the orchestrator would treat the
            // turn as continuable, start another assistant turn, and the
            // model would see the pending notice as an error and call a
            // different function — looping with no progress. The real
            // result lands in a future turn via approval::consume after
            // the operator resolves; approval-gate re-kicks the turn by
            // publishing `turn::step_requested` from `approval::resolve`.
            terminate: true,
        };
    }
    let denial_kind = merged
        .get("denial")
        .and_then(|d| d.get("kind"))
        .and_then(Value::as_str);
    let state_error = denial_kind == Some("state_error");
    let reason = merged
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("blocked")
        .to_string();
    FunctionResult {
        content: vec![ContentBlock::Text(TextContent { text: reason })],
        details: if state_error {
            json!({ "blocked": true, "denial": merged.get("denial").cloned().unwrap_or(Value::Null) })
        } else {
            json!({ "blocked": true })
        },
        // Fail-closed transport / state errors terminate the turn so the
        // model doesn't loop retrying against an unhealthy substrate.
        terminate: state_error,
    }
}

/// Synthetic block reply used when `publish_collect` exhausts retries.
/// Wire-compatible with what an `approval-gate` (or any other hook
/// subscriber) would have returned for a state-bus outage. The shape
/// reuses `Denial::StateError` so downstream stitching / audit treats
/// hook-bus failures identically to subscriber-side state-write failures.
pub(crate) fn fail_closed_block_reply(phase: &str, error: &str) -> Value {
    json!({
        "block": true,
        "status": "denied",
        "denial": {
            "kind": "state_error",
            "detail": {
                "phase": phase,
                "error": error,
            },
        },
        "reason": format!("hook bus unavailable during {phase}: {error}"),
    })
}

/// Map `tool_use {name: "agent_call", input: {function, payload}}` back to
/// a normal [`FunctionCall`] carrying the inner function id. Non-`agent_call`
/// calls pass through unchanged so legacy/test fixtures keep working.
fn unwrap_agent_call(fc: FunctionCall) -> FunctionCall {
    if fc.function_id != AGENT_CALL_TOOL_NAME {
        return fc;
    }
    let function = fc
        .arguments
        .get("function")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let payload = fc
        .arguments
        .get("payload")
        .cloned()
        .unwrap_or_else(|| json!({}));
    FunctionCall {
        id: fc.id,
        function_id: function,
        arguments: payload,
    }
}
pub async fn handle_prepare(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    record.function_results.clear();
    let raw = std::mem::take(&mut record.pending_function_calls);
    record.pending_function_calls = raw.into_iter().map(unwrap_agent_call).collect();

    // run_request is immutable for a session; loading it here on every retry of
    // FunctionPrepare is wasteful but correct. Cache on TurnStateRecord if hot.
    let run_request = persistence::load_run_request(iii, &record.session_id).await;
    let approval_required: Vec<String> = run_request
        .get("approval_required")
        .and_then(|v| match serde_json::from_value::<Vec<String>>(v.clone()) {
            Ok(list) => Some(list),
            Err(err) => {
                tracing::warn!(
                    %err,
                    session_id = %record.session_id,
                    "approval_required malformed in run_request; treating as empty"
                );
                None
            }
        })
        .unwrap_or_default();

    let mut prepared: Vec<(FunctionCall, Option<FunctionResult>)> =
        Vec::with_capacity(record.pending_function_calls.len());
    for fc in record.pending_function_calls.iter().cloned() {
        // Finding #1: a hook-bus outage used to silently fail OPEN —
        // `publish_collect` returned `{}`, `block` defaulted to false,
        // and the call ran without any subscriber having decided. We
        // now retry briefly; if that still fails we synthesize a
        // structured StateError block so the gate refuses the call and
        // the turn terminates cleanly.
        let merged = match publish_collect_with_retry(
            iii,
            TOPIC_BEFORE,
            build_before_function_call_payload(&record.session_id, &fc, &approval_required),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
            // require_reply=true: TOPIC_BEFORE is safety-critical. An
            // empty-replies response (subscriber down / slow / crashed)
            // must fail closed, not silently green-light the call.
            true,
        )
        .await
        {
            Ok(v) => v,
            Err(err) => {
                tracing::error!(
                    error = %err,
                    session_id = %record.session_id,
                    function_call_id = %fc.id,
                    function_id = %fc.function_id,
                    "hook bus unavailable on agent::before_function_call — failing closed",
                );
                fail_closed_block_reply("hook_publish", &err.to_string())
            }
        };
        let blocked = merged
            .get("block")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let prefilled = if blocked {
            Some(prefilled_result_for_block(&merged, &fc.id, &fc.function_id))
        } else {
            None
        };
        prepared.push((fc, prefilled));
    }

    persistence::save_record(iii, record).await;
    let executed = executed_staging_for_new_prepare_batch(&[]);
    persistence::save_executed_calls(iii, &record.session_id, &executed).await;
    persistence::save_prepared_calls(iii, &record.session_id, &prepared).await;

    record.transition_to(TurnState::FunctionExecute);
    Ok(())
}

pub async fn handle_execute(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    let prepared = persistence::load_prepared_calls(iii, &record.session_id).await;
    let mut results = persistence::load_executed_calls(iii, &record.session_id).await;
    for (fc, prefilled) in prepared {
        events::emit(
            iii,
            &record.session_id,
            &AgentEvent::FunctionExecutionStart {
                function_call_id: fc.id.clone(),
                function_id: fc.function_id.clone(),
                args: fc.arguments.clone(),
            },
        )
        .await;
        if let Some(blocked) = prefilled {
            // Pending approvals are an awaiting state, not an error — the
            // real result will be stitched on resolve. Only hard blocks
            // (deny / generic block) should render as red in the UI.
            let is_error = !blocked
                .details
                .get("pending_approval")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            persistence::upsert_executed_call(
                &mut results,
                (fc.clone(), blocked.clone(), is_error),
            );
            persistence::save_executed_calls(iii, &record.session_id, &results).await;
            let evt = build_function_execution_event(&fc, &blocked, is_error);
            events::emit(iii, &record.session_id, &evt).await;
            continue;
        }
        if let Some((_, recorded, recorded_is_error)) =
            persistence::find_executed_call(&results, &fc.id).cloned()
        {
            let evt = build_function_execution_event(&fc, &recorded, recorded_is_error);
            events::emit(iii, &record.session_id, &evt).await;
            continue;
        }
        let mut augmented = match fc.arguments.clone() {
            Value::Object(o) => Value::Object(o),
            other => json!({ "arguments": other }),
        };
        if let Some(obj) = augmented.as_object_mut() {
            obj.insert("session_id".into(), json!(record.session_id));
            obj.insert("function_call_id".into(), json!(fc.id));
            obj.insert("function_id".into(), json!(fc.function_id));
            obj.insert(
                "function_call".into(),
                json!({
                    "id": fc.id.clone(),
                    "function_id": fc.function_id.clone(),
                    "arguments": fc.arguments.clone(),
                }),
            );
        }

        let result = crate::agent_call::dispatch(
            iii,
            &record.session_id,
            &json!(fc.function_id.clone()),
            augmented,
        )
        .await;
        let is_error = result
            .details
            .get("error")
            .and_then(Value::as_str)
            .is_some();

        persistence::upsert_executed_call(&mut results, (fc.clone(), result.clone(), is_error));
        persistence::save_executed_calls(iii, &record.session_id, &results).await;
        let evt = build_function_execution_event(&fc, &result, is_error);
        events::emit(iii, &record.session_id, &evt).await;
    }
    record.transition_to(TurnState::FunctionFinalize);
    Ok(())
}

pub async fn handle_finalize(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    let executed = persistence::load_executed_calls(iii, &record.session_id).await;
    let mut function_results: Vec<FunctionResultMessage> = Vec::with_capacity(executed.len());
    let mut all_terminate = !executed.is_empty();
    for (fc, mut result, is_error) in executed {
        let merged = publish_collect_best_effort(
            iii,
            TOPIC_AFTER,
            json!({ "function_call": &fc, "result": &result }),
            "field_merge",
            HOOK_TIMEOUT_MS,
        )
        .await;
        if let Ok(after) = serde_json::from_value::<FunctionResult>(merged.clone()) {
            result = after;
        }
        if !result.terminate {
            all_terminate = false;
        }
        function_results.push(FunctionResultMessage {
            function_call_id: fc.id,
            function_id: fc.function_id,
            content: result.content,
            details: result.details,
            is_error,
            timestamp: chrono::Utc::now().timestamp_millis(),
        });
    }

    let mut messages = persistence::load_messages(iii, &record.session_id).await;
    let already_persisted: std::collections::HashSet<String> = messages
        .iter()
        .filter_map(|m| match m {
            AgentMessage::FunctionResult(fr) => Some(fr.function_call_id.clone()),
            _ => None,
        })
        .collect();
    let mut appended = false;
    for r in &function_results {
        if already_persisted.contains(&r.function_call_id) {
            // Two turn::step handlers can race when an approval re-kick
            // lands while the original chain is still finalizing — both
            // see state=FunctionFinalize and run this block. Without this
            // guard the persisted history grows a duplicate FunctionResult
            // per pending call, and the UI renders the same block twice.
            continue;
        }
        messages.push(AgentMessage::FunctionResult(r.clone()));
        appended = true;
    }
    if appended {
        persistence::save_messages(iii, &record.session_id, &messages).await;
    }

    let Some(last_assistant) = record.last_assistant.clone() else {
        tracing::warn!(
            session_id = %record.session_id,
            "FunctionFinalize reached without last_assistant; tearing down without lifecycle emit"
        );
        record.function_results = function_results;
        record.pending_function_calls.clear();
        record.transition_to(TurnState::TearingDown);
        return Ok(());
    };
    // Guard against the same race as the message-append above: emit
    // MessageStart/MessageEnd/TurnEnd only once even when a second
    // handler re-enters handle_finalize for the same turn.
    if !record.turn_end_emitted {
        for evt in build_finalize_lifecycle(&last_assistant, &function_results) {
            events::emit(iii, &record.session_id, &evt).await;
        }
        record.turn_end_emitted = true;
    }

    record.function_results = function_results;
    record.pending_function_calls.clear();
    if all_terminate {
        record.transition_to(TurnState::TearingDown);
    } else {
        record.transition_to(TurnState::SteeringCheck);
    }
    Ok(())
}

pub(crate) fn executed_staging_for_new_prepare_batch(
    _stale: &[(FunctionCall, FunctionResult, bool)],
) -> Vec<(FunctionCall, FunctionResult, bool)> {
    Vec::new()
}

/// Pure helper: inner payload for the `agent::before_function_call` topic.
///
/// `session_id` is included because `approval-gate::extract_call` requires
/// it to key the pending record; without it the gate silently passes every
/// call through.
pub(crate) fn build_before_function_call_payload(
    session_id: &str,
    fc: &FunctionCall,
    approval_required: &[String],
) -> Value {
    json!({
        "session_id": session_id,
        "function_call": fc,
        "approval_required": approval_required,
    })
}

/// Pure helper: build [`AgentEvent::FunctionExecutionEnd`] for one call.
pub(crate) fn build_function_execution_event(
    fc: &FunctionCall,
    result: &FunctionResult,
    is_error: bool,
) -> AgentEvent {
    AgentEvent::FunctionExecutionEnd {
        function_call_id: fc.id.clone(),
        function_id: fc.function_id.clone(),
        is_error,
        result: result.clone(),
    }
}

/// Lifecycle events at the end of a function-bearing turn.
pub(crate) fn build_finalize_lifecycle(
    assistant: &AssistantMessage,
    function_results: &[FunctionResultMessage],
) -> Vec<AgentEvent> {
    let mut out = Vec::with_capacity(function_results.len() * 2 + 1);
    for r in function_results {
        let m = AgentMessage::FunctionResult(r.clone());
        out.push(AgentEvent::MessageStart { message: m.clone() });
        out.push(AgentEvent::MessageEnd { message: m });
    }
    out.push(AgentEvent::TurnEnd {
        message: AgentMessage::Assistant(assistant.clone()),
        function_results: function_results.to_vec(),
    });
    out
}

/// Bounded-retry publisher used by `handle_prepare`. Returns `Err` only
/// after every attempt fails — callers fail-closed on `Err` (finding #1).
/// Retries: 2 attempts total, fixed 100ms gap. Generous enough to ride
/// over a ws blip; short enough that a real outage doesn't add seconds
/// of latency to every call.
///
/// Minimal abstraction over the single `iii.trigger("hook-fanout::publish_collect", ...)`
/// call. Production code uses the blanket [`III`] impl; tests inject a
/// fake to drive the retry helper through specific response shapes
/// (publish_ok=false, transient failure followed by success, …)
/// without booting a real iii engine.
#[async_trait::async_trait]
pub(crate) trait HookPublisher: Send + Sync {
    async fn publish_collect(&self, payload: Value) -> Result<Value, String>;
}

#[async_trait::async_trait]
impl HookPublisher for III {
    async fn publish_collect(&self, payload: Value) -> Result<Value, String> {
        self.trigger(TriggerRequest {
            function_id: "hook-fanout::publish_collect".into(),
            payload,
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| e.to_string())
    }
}

/// Two failure modes are treated identically and both retry:
///   1. `iii.trigger` itself returns `Err` (the bus refused the call).
///   2. `iii.trigger` returns `Ok` but the response body shows
///      `publish.ok == false` (hook-fanout's publish to subscribers
///      failed; without surfacing this the orchestrator can't tell a
///      broken substrate from a green "no subscriber blocked" merge —
///      finding #1 follow-up).
async fn publish_collect_with_retry<P: HookPublisher + ?Sized>(
    publisher: &P,
    topic: &str,
    inner: Value,
    merge_rule: &str,
    timeout_ms: u64,
    require_reply: bool,
) -> anyhow::Result<Value> {
    const ATTEMPTS: u32 = 2;
    const BACKOFF_MS: u64 = 100;
    let payload = json!({
        "topic": topic,
        "payload": inner,
        "merge_rule": merge_rule,
        "timeout_ms": timeout_ms,
    });
    let mut last_err: Option<String> = None;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(BACKOFF_MS)).await;
        }
        match publisher.publish_collect(payload.clone()).await {
            Ok(v) => {
                if let Some(err) = publish_failure_from_response(&v, require_reply) {
                    tracing::warn!(error = %err, attempt, topic, require_reply, "hook-fanout failure surface");
                    last_err = Some(err);
                    continue;
                }
                return Ok(v.get("merged").cloned().unwrap_or_else(|| json!({})));
            }
            Err(e) => {
                tracing::warn!(error = %e, attempt, topic, "hook-fanout::publish_collect failed");
                last_err = Some(e);
            }
        }
    }
    Err(anyhow::anyhow!(
        last_err.unwrap_or_else(|| "publish_collect failed".to_string())
    ))
}

/// Inspect a `hook-fanout::publish_collect` response for a fail-closed
/// signal. Returns `Some(error_message)` when the response must be
/// treated as a transport-level failure; `None` when the response is
/// a healthy decision.
///
/// Three failure shapes are detected:
///   1. Modern `publish: { ok: false, error: "..." }` (durable publish
///      failed end-to-end).
///   2. Legacy top-level `publish_failed: true` (hook-fanout pre-modern
///      shape; drop after one release).
///   3. **Zero-replies on a safety-critical caller**: when
///      `require_reply` is true (used by `handle_prepare` for the
///      `agent::before_function_call` hook), an Ok response whose
///      `replies` array is empty means "publish succeeded, but no
///      subscriber actually decided." Without this branch the gate
///      fails OPEN any time approval-gate is unsubscribed, slow, or
///      crashed mid-handle — `merge_first_block_wins([])` yields
///      `{block: false}` indistinguishable from a green policy
///      decision.
///
/// Pure helper — unit tested below. Centralizing this lets the retry
/// loop fail-closed without re-implementing the response schema.
pub(crate) fn publish_failure_from_response(v: &Value, require_reply: bool) -> Option<String> {
    // Modern shape: `publish: { ok: false, error: "..." }`.
    if let Some(publish) = v.get("publish") {
        if publish.get("ok").and_then(Value::as_bool) == Some(false) {
            return Some(
                publish
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("publish failed")
                    .to_string(),
            );
        }
    }
    // Legacy shape: top-level `publish_failed: true`. Drop after one
    // release once every hook-fanout in the fleet emits `publish`.
    if v.get("publish_failed").and_then(Value::as_bool) == Some(true) {
        return Some("publish failed (legacy publish_failed flag)".to_string());
    }
    // Zero-replies on a safety-critical caller — see doc-comment.
    if require_reply {
        let reply_count = v
            .get("replies")
            .and_then(Value::as_array)
            .map(|a| a.len())
            .unwrap_or(0);
        if reply_count == 0 {
            return Some(
                "publish succeeded but no subscriber replied — failing closed".to_string(),
            );
        }
    }
    None
}

/// Best-effort publisher used by `handle_finalize` (the AFTER hook). The
/// function has already executed by the time this fires, so a bus blip
/// must not terminate the turn — we log and fall back to the prior
/// result. Different policy from `handle_prepare`; see finding #1.
async fn publish_collect_best_effort(
    iii: &III,
    topic: &str,
    inner: Value,
    merge_rule: &str,
    timeout_ms: u64,
) -> Value {
    // require_reply=false: after-hooks with zero subscribers are a
    // legitimate state (no observers registered for the topic). We
    // only need to fail when the publish itself broke.
    match publish_collect_with_retry(iii, topic, inner, merge_rule, timeout_ms, false).await {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                error = %err,
                topic,
                "after-hook publish failed; keeping prior result",
            );
            json!({})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_types::{AgentEvent, AssistantMessage, ContentBlock, FunctionCall, TextContent};

    fn fc(id: &str, function_id: &str, args: serde_json::Value) -> FunctionCall {
        FunctionCall {
            id: id.into(),
            function_id: function_id.into(),
            arguments: args,
        }
    }

    #[test]
    fn standard_agent_call_unwraps_to_inner() {
        let input = fc(
            "call_1",
            "agent_call",
            json!({ "function": "shell::fs::ls", "payload": { "path": "/tmp" } }),
        );
        let out = unwrap_agent_call(input);
        assert_eq!(out.id, "call_1");
        assert_eq!(out.function_id, "shell::fs::ls");
        assert_eq!(out.arguments, json!({ "path": "/tmp" }));
    }

    #[test]
    fn missing_payload_defaults_to_empty_object() {
        let input = fc(
            "call_2",
            "agent_call",
            json!({ "function": "directory::skills::list" }),
        );
        let out = unwrap_agent_call(input);
        assert_eq!(out.function_id, "directory::skills::list");
        assert_eq!(out.arguments, json!({}));
    }

    #[test]
    fn non_agent_call_returns_unchanged() {
        let input = fc("call_3", "shell::fs::ls", json!({ "path": "/tmp" }));
        let out = unwrap_agent_call(input.clone());
        assert_eq!(out, input);
    }

    #[test]
    fn missing_function_field_unwraps_to_empty_function_id() {
        let input = fc("call_4", "agent_call", json!({ "payload": { "x": 1 } }));
        let out = unwrap_agent_call(input);
        assert_eq!(out.function_id, "");
        assert_eq!(out.arguments, json!({ "x": 1 }));
    }

    #[test]
    fn unwrapped_calls_replace_agent_call_in_place() {
        let calls = vec![
            fc(
                "a",
                "agent_call",
                json!({"function":"shell::fs::ls","payload":{"path":"/tmp"}}),
            ),
            fc("b", "directory::skills::list", json!({})),
        ];
        let unwrapped: Vec<_> = calls.into_iter().map(unwrap_agent_call).collect();
        assert_eq!(unwrapped[0].function_id, "shell::fs::ls");
        assert_eq!(unwrapped[0].arguments, json!({"path":"/tmp"}));
        assert_eq!(unwrapped[1].function_id, "directory::skills::list");
    }

    /// REGRESSION: `handle_execute` must route through `agent_call::dispatch`.
    #[test]
    fn handle_execute_does_not_call_iii_trigger_with_fc_name_directly() {
        let src = include_str!("functions.rs");
        let start = src
            .find("pub async fn handle_execute")
            .expect("handle_execute exists");
        let window = &src[start..start + src[start..].len().min(5000)];
        assert!(
            window.contains("agent_call::dispatch"),
            "handle_execute must call agent_call::dispatch"
        );
    }

    fn assistant_with_function_call(function_id: &str) -> AssistantMessage {
        AssistantMessage {
            content: vec![ContentBlock::FunctionCall {
                id: "tc-1".into(),
                function_id: function_id.into(),
                arguments: json!({}),
            }],
            stop_reason: harness_types::StopReason::FunctionCall,
            error_message: None,
            error_kind: None,
            usage: None,
            model: "m".into(),
            provider: "p".into(),
            timestamp: 0,
        }
    }

    fn function_result_msg(function_id: &str, is_error: bool) -> FunctionResultMessage {
        FunctionResultMessage {
            function_call_id: "tc-1".into(),
            function_id: function_id.into(),
            content: vec![ContentBlock::Text(TextContent {
                text: "done".into(),
            })],
            details: json!({}),
            is_error,
            timestamp: 0,
        }
    }

    #[test]
    fn new_prepare_batch_clears_stale_executed_call_ids() {
        let stale = vec![(
            FunctionCall {
                id: "tc-1".into(),
                function_id: "read".into(),
                arguments: json!({}),
            },
            FunctionResult {
                content: vec![],
                details: json!({}),
                terminate: false,
            },
            false,
        )];

        let reset = executed_staging_for_new_prepare_batch(&stale);

        assert!(persistence::find_executed_call(&stale, "tc-1").is_some());
        assert!(persistence::find_executed_call(&reset, "tc-1").is_none());
        assert!(reset.is_empty());
    }

    #[test]
    fn build_function_execution_event_carries_function_id_and_error_flag() {
        let fc = FunctionCall {
            id: "tc-1".into(),
            function_id: "read".into(),
            arguments: json!({"path": "/tmp/x"}),
        };
        let result = FunctionResult {
            content: vec![ContentBlock::Text(TextContent { text: "ok".into() })],
            details: json!({}),
            terminate: false,
        };
        let evt = build_function_execution_event(&fc, &result, false);
        match evt {
            AgentEvent::FunctionExecutionEnd {
                function_id,
                is_error,
                ..
            } => {
                assert_eq!(function_id, "read");
                assert!(!is_error);
            }
            other => panic!("expected FunctionExecutionEnd, got {other:?}"),
        }
    }

    #[test]
    fn build_function_execution_event_marks_blocked_as_error() {
        let fc = FunctionCall {
            id: "tc-2".into(),
            function_id: "bash".into(),
            arguments: json!({"command": "rm -rf /"}),
        };
        let blocked = FunctionResult {
            content: vec![ContentBlock::Text(TextContent {
                text: "blocked by policy".into(),
            })],
            details: json!({"blocked": true}),
            terminate: false,
        };
        let evt = build_function_execution_event(&fc, &blocked, true);
        match evt {
            AgentEvent::FunctionExecutionEnd {
                function_id,
                is_error,
                result,
                ..
            } => {
                assert_eq!(function_id, "bash");
                assert!(is_error);
                assert!(matches!(
                    result.content.first(),
                    Some(ContentBlock::Text(t)) if t.text == "blocked by policy"
                ));
            }
            other => panic!("expected FunctionExecutionEnd, got {other:?}"),
        }
    }

    #[test]
    fn before_function_call_payload_carries_approval_required() {
        let fc = FunctionCall {
            id: "tc-1".into(),
            function_id: "shell::fs::write".into(),
            arguments: json!({"path": "/tmp/x"}),
        };
        let approval_required = vec!["shell::fs::write".to_string()];
        let inner = build_before_function_call_payload("sess-a", &fc, &approval_required);
        assert_eq!(inner["function_call"]["id"], "tc-1");
        assert_eq!(inner["approval_required"], json!(["shell::fs::write"]),);
    }

    #[test]
    fn before_function_call_payload_has_empty_approval_required_when_none_configured() {
        let fc = FunctionCall {
            id: "tc-1".into(),
            function_id: "shell::fs::ls".into(),
            arguments: json!({}),
        };
        let inner = build_before_function_call_payload("sess-a", &fc, &[]);
        assert_eq!(inner["approval_required"], json!([]));
    }

    /// Regression: the approval-gate subscriber requires `session_id` to key
    /// its pending record. Dropping it from the payload makes the gate
    /// silently pass every call through.
    #[test]
    fn before_function_call_payload_includes_session_id() {
        let fc = FunctionCall {
            id: "tc-1".into(),
            function_id: "shell::fs::write".into(),
            arguments: json!({}),
        };
        let inner = build_before_function_call_payload("sess-xyz", &fc, &[]);
        assert_eq!(inner["session_id"], "sess-xyz");
    }

    #[test]
    fn build_finalize_lifecycle_emits_pair_per_result_then_turn_end() {
        let asst = assistant_with_function_call("read");
        let results = vec![
            function_result_msg("read", false),
            function_result_msg("write", false),
        ];
        let evs = build_finalize_lifecycle(&asst, &results);
        assert_eq!(evs.len(), 5);
        assert!(matches!(&evs[0], AgentEvent::MessageStart { .. }));
        assert!(matches!(evs.last(), Some(AgentEvent::TurnEnd { .. })));
    }

    /// policy-denylist subscribes to this topic by exact name.
    #[test]
    fn topic_constants_are_stable() {
        assert_eq!(TOPIC_BEFORE, "agent::before_function_call");
        assert_eq!(TOPIC_AFTER, "agent::after_function_call");
    }

    /// `function_call.function_id` is what `policy-denylist` matches against
    /// `POLICY_DENIED_FUNCTIONS`.
    #[test]
    fn build_before_function_call_payload_preserves_function_call_shape() {
        let fc = FunctionCall {
            id: "tc-1".into(),
            function_id: "shell::fs::ls".into(),
            arguments: json!({"path": "/tmp"}),
        };
        let inner = build_before_function_call_payload("sess-a", &fc, &[]);
        assert_eq!(inner["function_call"]["id"], "tc-1");
        assert_eq!(inner["function_call"]["function_id"], "shell::fs::ls");
        assert_eq!(inner["function_call"]["arguments"], json!({"path": "/tmp"}));
        assert!(inner.get("approval_required").is_some());
    }

    #[test]
    fn handle_finalize_does_not_expect_last_assistant() {
        let src = include_str!("functions.rs");
        let start = src
            .find("pub async fn handle_finalize")
            .expect("handle_finalize exists");
        let window = &src[start..start + src[start..].len().min(3000)];
        assert!(
            !window.contains(".expect(\"tools state requires last_assistant"),
            "handle_finalize must not .expect() last_assistant"
        );
    }

    #[test]
    fn prefilled_result_for_block_pending_includes_call_id_in_details() {
        let merged = json!({
            "block": true, "status": "pending",
            "call_id": "tc-1", "function_id": "shell::fs::write",
        });
        let fr = prefilled_result_for_block(&merged, "tc-1", "shell::fs::write");
        assert_eq!(fr.details["pending_approval"], json!(true));
        assert_eq!(fr.details["call_id"], json!("tc-1"));
        let text = match &fr.content[0] {
            ContentBlock::Text(t) => &t.text,
            _ => panic!("expected text block"),
        };
        assert!(text.contains("pending_approval"));
        assert!(text.contains("tc-1"));
        // Stop the auto-loop: pending must terminate the turn so the
        // orchestrator waits for approval::resolve → re-kick.
        assert!(
            fr.terminate,
            "pending_approval result must terminate the turn"
        );
    }

    #[test]
    fn prefilled_result_for_block_non_pending_uses_plain_reason() {
        let merged = json!({"block": true, "reason": "policy::no"});
        let fr = prefilled_result_for_block(&merged, "tc-1", "shell::fs::write");
        assert_eq!(fr.details, json!({"blocked": true}));
        let text = match &fr.content[0] {
            ContentBlock::Text(t) => &t.text,
            _ => panic!("expected text block"),
        };
        assert_eq!(text, "policy::no");
        assert!(
            !fr.terminate,
            "structured policy deny lets the model try something else",
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // Finding #1: hook-bus failure must fail CLOSED.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn fail_closed_block_reply_uses_state_error_shape() {
        let v = fail_closed_block_reply("hook_publish", "ws disconnected");
        assert_eq!(v["block"], true);
        assert_eq!(v["status"], "denied");
        assert_eq!(v["denial"]["kind"], "state_error");
        assert_eq!(v["denial"]["detail"]["phase"], "hook_publish");
        assert_eq!(v["denial"]["detail"]["error"], "ws disconnected");
        assert!(v["reason"]
            .as_str()
            .is_some_and(|s| s.contains("hook bus unavailable")));
    }

    #[test]
    fn prefilled_result_for_state_error_terminates_the_turn() {
        // Failing closed must stop the loop — otherwise the model would
        // keep retrying against an unhealthy substrate and could bypass
        // the gate once a subscriber recovers mid-turn.
        let merged = fail_closed_block_reply("hook_publish", "boom");
        let fr = prefilled_result_for_block(&merged, "tc-1", "shell::exec");
        assert!(
            fr.terminate,
            "state_error block must terminate the turn (finding #1)",
        );
        assert_eq!(fr.details["denial"]["kind"], "state_error");
        let text = match &fr.content[0] {
            ContentBlock::Text(t) => &t.text,
            _ => panic!("expected text block"),
        };
        assert!(text.contains("hook bus unavailable"));
    }

    // ──────────────────────────────────────────────────────────────────
    // Finding #1 follow-up: hook-fanout can return Ok with publish_failed.
    // The retry helper MUST treat that as a transient error, not a green
    // "no subscriber blocked" decision.
    //
    // The behavioral coverage lives in the `publish_collect_with_retry_*`
    // tests further down — they drive the helper through a fake
    // [`HookPublisher`] and assert the Err/Ok outcomes that `handle_prepare`
    // relies on. The earlier string-scan regression tests were redundant
    // once those exist and were removed.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn publish_failure_from_response_detects_modern_publish_block() {
        let v = json!({
            "event_id": "e1",
            "replies": [],
            "merged": { "block": false },
            "publish": { "ok": false, "error": "ws closed" },
        });
        assert_eq!(
            publish_failure_from_response(&v, false).as_deref(),
            Some("ws closed"),
        );
    }

    #[test]
    fn publish_failure_from_response_detects_legacy_top_level_flag() {
        let v = json!({
            "merged": { "block": false },
            "publish_failed": true,
        });
        assert!(
            publish_failure_from_response(&v, false).is_some(),
            "legacy `publish_failed: true` must still trigger fail-closed",
        );
    }

    #[test]
    fn publish_failure_from_response_returns_none_on_success() {
        let v = json!({
            "merged": { "block": false },
            "publish": { "ok": true },
            "replies": [{ "block": false }],
        });
        assert!(publish_failure_from_response(&v, false).is_none());
        assert!(
            publish_failure_from_response(&v, true).is_none(),
            "require_reply=true still passes when ≥1 reply exists",
        );
    }

    /// Critical safety property: a successful trigger that returned an
    /// empty-merge response WITHOUT a publish marker is treated as a
    /// real green decision when require_reply=false. The same shape
    /// must fail closed when require_reply=true (zero-replies path).
    #[test]
    fn publish_failure_from_response_treats_absent_marker_as_success_lenient() {
        let v = json!({
            "merged": { "block": false },
            // No publish block; older fleet shape pre-fix.
        });
        assert!(
            publish_failure_from_response(&v, false).is_none(),
            "lenient caller (after-hook) treats absent marker as success",
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // NEW finding (P1): zero-replies on a safety-critical caller must
    // fail closed. Pre-fix the helper returned None here and the gate
    // green-lit the call when approval-gate was unsubscribed or slow.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn publish_failure_from_response_fails_closed_on_empty_replies_when_required() {
        let v = json!({
            "event_id": "e1",
            "replies": [],
            "merged": { "block": false },
            "publish": { "ok": true },
        });
        assert!(
            publish_failure_from_response(&v, true).is_some(),
            "safety-critical caller (TOPIC_BEFORE) must fail closed when \
             no subscriber replied — `merge_first_block_wins([])` yields \
             {{block:false}} that is indistinguishable from a green policy decision",
        );
    }

    #[test]
    fn publish_failure_from_response_allows_zero_replies_when_not_required() {
        let v = json!({
            "event_id": "e1",
            "replies": [],
            "merged": { "block": false },
            "publish": { "ok": true },
        });
        assert!(
            publish_failure_from_response(&v, false).is_none(),
            "after-hook (require_reply=false) tolerates zero subscribers",
        );
    }

    #[test]
    fn publish_failure_from_response_passes_when_any_reply_landed_with_required() {
        let v = json!({
            "event_id": "e1",
            "replies": [{ "block": false }],
            "merged": { "block": false },
            "publish": { "ok": true },
        });
        assert!(
            publish_failure_from_response(&v, true).is_none(),
            "one healthy reply satisfies require_reply=true (approval-gate said allow)",
        );
    }

    #[test]
    fn publish_failure_from_response_missing_replies_field_treated_as_empty() {
        // Defensive: if hook-fanout drops the `replies` field entirely
        // we MUST treat that as zero replies, not as "field absent ⇒
        // unknown ⇒ assume success". Reading the field as 0 is the
        // safe default for the safety-critical caller.
        let v = json!({
            "merged": { "block": false },
            "publish": { "ok": true },
        });
        assert!(
            publish_failure_from_response(&v, true).is_some(),
            "missing replies field must be treated as zero replies under require_reply",
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // Behavioral coverage of `publish_collect_with_retry` (finding #1).
    //
    // `FakePublisher` drives canned responses into the helper and counts
    // call attempts. Each test pins one branch of the
    // succeed / retry / fail-closed matrix. With these in place the gate
    // can't silently fail open if some future change drops the
    // `publish_failure_from_response` check, ignores Err returns from
    // the trigger call, or skips the retry on transient failure.
    // ──────────────────────────────────────────────────────────────────

    /// Test-only [`HookPublisher`] that walks through a canned script of
    /// responses (one per attempt) and records the number of calls. Once
    /// the script runs out the final entry is repeated indefinitely so
    /// tests can assert ATTEMPTS-bounded behavior without hard-coding the
    /// attempt count.
    struct FakePublisher {
        script: std::sync::Mutex<Vec<Result<Value, String>>>,
        calls: std::sync::Mutex<u32>,
    }

    impl FakePublisher {
        fn new(script: Vec<Result<Value, String>>) -> Self {
            Self {
                script: std::sync::Mutex::new(script),
                calls: std::sync::Mutex::new(0),
            }
        }
        fn call_count(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl HookPublisher for FakePublisher {
        async fn publish_collect(&self, _payload: Value) -> Result<Value, String> {
            *self.calls.lock().unwrap() += 1;
            let mut script = self.script.lock().unwrap();
            if script.len() > 1 {
                script.remove(0)
            } else {
                script.first().cloned().unwrap_or_else(|| {
                    Err("FakePublisher script empty".to_string())
                })
            }
        }
    }

    /// The fail-closed contract: when hook-fanout reports `publish.ok=false`
    /// on every attempt the helper must surface that as `Err` so
    /// `handle_prepare` synthesizes a state_error block. Pre-f4ee90d the
    /// helper returned `Ok(json!({block:false}))` and the gate failed
    /// OPEN.
    #[tokio::test]
    async fn publish_collect_with_retry_returns_err_when_publish_ok_false() {
        let publisher = FakePublisher::new(vec![Ok(json!({
            "merged": { "block": false },
            "publish": { "ok": false, "error": "ws closed" },
        }))]);
        let out = publish_collect_with_retry(
            &publisher,
            TOPIC_BEFORE,
            json!({}),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
            false,
        )
        .await;
        let err = out.expect_err("publish.ok=false must surface as Err");
        assert!(
            err.to_string().contains("ws closed"),
            "Err must carry hook-fanout's diagnostic, got {err}",
        );
        assert!(
            publisher.call_count() >= 2,
            "fail-closed path must retry, got {} calls",
            publisher.call_count(),
        );
    }

    /// Legacy hook-fanout fleet emits `publish_failed: true` at the top
    /// level instead of the modern `publish: { ok: false }`. The helper
    /// must still fail closed.
    #[tokio::test]
    async fn publish_collect_with_retry_returns_err_on_legacy_publish_failed_flag() {
        let publisher = FakePublisher::new(vec![Ok(json!({
            "merged": { "block": false },
            "publish_failed": true,
        }))]);
        let out = publish_collect_with_retry(
            &publisher,
            TOPIC_BEFORE,
            json!({}),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
            false,
        )
        .await;
        assert!(
            out.is_err(),
            "legacy publish_failed=true must surface as Err",
        );
    }

    /// Happy path: when hook-fanout returns a healthy response the helper
    /// returns just the `merged` sub-object so `handle_prepare` can read
    /// `block` / `reason` directly.
    #[tokio::test]
    async fn publish_collect_with_retry_returns_merged_on_success() {
        let publisher = FakePublisher::new(vec![Ok(json!({
            "merged": { "block": true, "reason": "policy::no" },
            "publish": { "ok": true },
        }))]);
        let merged = publish_collect_with_retry(
            &publisher,
            TOPIC_BEFORE,
            json!({}),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
            false,
        )
        .await
        .expect("healthy response must succeed");
        assert_eq!(merged, json!({ "block": true, "reason": "policy::no" }));
        assert_eq!(publisher.call_count(), 1, "success must not retry");
    }

    /// Transient failure recovers: first attempt reports publish.ok=false,
    /// second attempt succeeds → helper returns Ok with the second
    /// merged payload.
    #[tokio::test]
    async fn publish_collect_with_retry_retries_on_transient_publish_failure() {
        let publisher = FakePublisher::new(vec![
            Ok(json!({
                "merged": { "block": false },
                "publish": { "ok": false, "error": "transient" },
            })),
            Ok(json!({
                "merged": { "block": false },
                "publish": { "ok": true },
            })),
        ]);
        let merged = publish_collect_with_retry(
            &publisher,
            TOPIC_BEFORE,
            json!({}),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
            false,
        )
        .await
        .expect("second attempt must succeed");
        assert_eq!(merged, json!({ "block": false }));
        assert_eq!(
            publisher.call_count(),
            2,
            "must retry exactly once on transient failure",
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // NEW finding (P1): zero-replies fail-closed at the helper level.
    // Pre-fix, the helper accepted any `publish.ok=true` response —
    // even with empty replies — and `merge_first_block_wins([])`
    // produced `{block:false}`. The retry helper must now treat that
    // exact response as a transient failure when called by the safety-
    // critical path (TOPIC_BEFORE / handle_prepare), and retry / surface
    // Err just like a publish-level failure.
    // ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn publish_collect_with_retry_fails_closed_on_zero_replies_when_required() {
        let publisher = FakePublisher::new(vec![Ok(json!({
            "event_id": "e1",
            "replies": [],
            "merged": { "block": false },
            "publish": { "ok": true },
        }))]);
        let out = publish_collect_with_retry(
            &publisher,
            TOPIC_BEFORE,
            json!({}),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
            true,
        )
        .await;
        let err = out.expect_err(
            "require_reply=true: zero replies must surface as Err, not green-light the call",
        );
        assert!(
            err.to_string().to_lowercase().contains("no subscriber"),
            "Err must explain why we failed closed (got: {err})",
        );
        assert!(
            publisher.call_count() >= 2,
            "zero-replies path must retry like any other transient failure, got {} calls",
            publisher.call_count(),
        );
    }

    #[tokio::test]
    async fn publish_collect_with_retry_tolerates_zero_replies_when_not_required() {
        let publisher = FakePublisher::new(vec![Ok(json!({
            "event_id": "e1",
            "replies": [],
            "merged": { "block": false },
            "publish": { "ok": true },
        }))]);
        let merged = publish_collect_with_retry(
            &publisher,
            TOPIC_AFTER,
            json!({}),
            "field_merge",
            HOOK_TIMEOUT_MS,
            false,
        )
        .await
        .expect("after-hook with zero subscribers is a legitimate green state");
        assert_eq!(merged, json!({ "block": false }));
        assert_eq!(
            publisher.call_count(),
            1,
            "require_reply=false must NOT retry on empty replies (no transient signal)",
        );
    }

    #[tokio::test]
    async fn publish_collect_with_retry_recovers_when_subscriber_replies_on_second_attempt() {
        let publisher = FakePublisher::new(vec![
            Ok(json!({
                "event_id": "e1",
                "replies": [],
                "merged": { "block": false },
                "publish": { "ok": true },
            })),
            Ok(json!({
                "event_id": "e1",
                "replies": [{ "block": true, "status": "pending" }],
                "merged": { "block": true, "status": "pending" },
                "publish": { "ok": true },
            })),
        ]);
        let merged = publish_collect_with_retry(
            &publisher,
            TOPIC_BEFORE,
            json!({}),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
            true,
        )
        .await
        .expect("recovery on second attempt must succeed");
        assert_eq!(merged["block"], true);
        assert_eq!(merged["status"], "pending");
        assert_eq!(publisher.call_count(), 2);
    }

    /// REGRESSION: `handle_prepare` MUST use require_reply=true. A revert
    /// to false would silently bring back the zero-replies fail-open
    /// path. Source-level scan catches that without needing to drive a
    /// full orchestrator turn.
    #[test]
    fn handle_prepare_passes_require_reply_true() {
        let src = include_str!("functions.rs");
        let start = src
            .find("pub async fn handle_prepare")
            .expect("handle_prepare exists");
        let window = &src[start..start + src[start..].len().min(4000)];
        // Search inside the prepare body for the require_reply argument
        // on the call to publish_collect_with_retry.
        assert!(
            window.contains("// require_reply=true"),
            "handle_prepare must pass require_reply=true to publish_collect_with_retry \
             (fail-closed on zero-replies path)",
        );
    }

    /// Trigger-level Err (the bus refused the call) is also retried. Two
    /// straight Errs surface as Err.
    #[tokio::test]
    async fn publish_collect_with_retry_returns_err_when_trigger_errors() {
        let publisher = FakePublisher::new(vec![
            Err("bus down".to_string()),
            Err("bus still down".to_string()),
        ]);
        let out = publish_collect_with_retry(
            &publisher,
            TOPIC_BEFORE,
            json!({}),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
            false,
        )
        .await;
        let err = out.expect_err("trigger Err must surface");
        assert!(
            err.to_string().contains("bus"),
            "Err must carry the last trigger error",
        );
        assert_eq!(publisher.call_count(), 2);
    }

    /// REGRESSION: `handle_finalize`'s AFTER hook must NOT terminate the
    /// turn on publish failure — the function already ran; we keep the
    /// prior result and log.
    #[test]
    fn handle_finalize_uses_best_effort_after_hook() {
        let src = include_str!("functions.rs");
        let start = src
            .find("pub async fn handle_finalize")
            .expect("handle_finalize exists");
        let window = &src[start..start + src[start..].len().min(2500)];
        assert!(
            window.contains("publish_collect_best_effort"),
            "handle_finalize must use the best-effort publisher (no fail-closed)",
        );
        assert!(
            !window.contains("fail_closed_block_reply"),
            "handle_finalize must not fail closed — the function already ran",
        );
    }
}
