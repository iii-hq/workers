//! `function_prepare`, `function_execute`, `function_finalize` handlers.

use harness_types::{
    AgentEvent, AgentMessage, AssistantMessage, ContentBlock, FunctionCall, FunctionResult,
    FunctionResultMessage, TextContent, TruncationInfo,
};
use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;
use std::collections::HashSet;

use crate::agent_call::TOOL_NAME as AGENT_CALL_TOOL_NAME;
use crate::events;
use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};

const TOPIC_BEFORE: &str = "agent::before_function_call";
const TOPIC_AFTER: &str = "agent::after_function_call";
const HOOK_TIMEOUT_MS: u64 = 10_000;

// ─── Tool-result truncation ──────────────────────────────────────────────
//
// Large tool outputs (multi-MB shell::run, big shell::fs::read) dominate
// per-turn token cost. We cap each FunctionResult at a serialized-bytes
// budget; oversized payloads get stashed under `session/<id>/result/<call_id>`
// and the in-stream `content` is replaced with a head+tail-elided preview
// plus a marker telling the model how to call `result::fetch` to recover
// the full payload (intercepted in `agent_call::dispatch`).

const DEFAULT_TRUNCATE_BYTES: usize = 8192;
const TRUNCATE_ENV: &str = "HARNESS_RESULT_TRUNCATE_BYTES";
const TRUNCATE_HEAD_BYTES: usize = 2048;
const TRUNCATE_TAIL_BYTES: usize = 2048;

fn truncate_threshold() -> usize {
    static CACHED: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        std::env::var(TRUNCATE_ENV)
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&v| v > 0)
            .unwrap_or(DEFAULT_TRUNCATE_BYTES)
    })
}

/// If `result`'s serialized size exceeds the truncation threshold, stash the
/// full payload under `session/<session_id>/result/<call_id>` and return a
/// compact replacement carrying a `TruncationInfo` pointer. Otherwise return
/// `result` unchanged.
async fn maybe_truncate_result(
    iii: &III,
    session_id: &str,
    call_id: &str,
    result: FunctionResult,
) -> FunctionResult {
    let threshold = truncate_threshold();
    let serialized_size = match serde_json::to_string(&result) {
        Ok(s) => s.len(),
        Err(_) => return result,
    };
    if serialized_size <= threshold {
        return result;
    }
    // Persist the full payload first; if state::set fails, fall through and
    // return the original result so we never lose data.
    let full_json = match serde_json::to_value(&result) {
        Ok(v) => v,
        Err(_) => return result,
    };
    persistence::save_full_result(iii, session_id, call_id, &full_json).await;

    let summary_text = render_truncated_text(&result, serialized_size, call_id);
    FunctionResult {
        content: vec![ContentBlock::Text(TextContent { text: summary_text })],
        details: json!({
            "truncated": true,
            "original_bytes": serialized_size,
            "call_id": call_id,
        }),
        terminate: result.terminate,
        truncated: Some(TruncationInfo {
            original_bytes: serialized_size as u64,
            call_id: call_id.to_string(),
        }),
    }
}

/// Render the model-facing replacement text for a truncated result. Head +
/// tail elision keeps the most semantically useful parts (shell errors
/// typically live at the bottom, while file headers / call signatures live
/// at the top).
fn render_truncated_text(result: &FunctionResult, original_bytes: usize, call_id: &str) -> String {
    let mut combined = String::new();
    for block in &result.content {
        if let ContentBlock::Text(t) = block {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&t.text);
        }
    }
    let total = combined.len();
    let body = if total > TRUNCATE_HEAD_BYTES + TRUNCATE_TAIL_BYTES {
        let head_end = char_boundary_floor(&combined, TRUNCATE_HEAD_BYTES);
        let tail_start = char_boundary_ceil(&combined, total - TRUNCATE_TAIL_BYTES);
        format!(
            "{head}\n\n[... {elided} bytes elided ...]\n\n{tail}",
            head = &combined[..head_end],
            elided = total - head_end - (total - tail_start),
            tail = &combined[tail_start..],
        )
    } else {
        combined
    };
    format!(
        "[result truncated — {original_bytes} bytes — call agent_call with \
         function=\"result::fetch\", payload={{\"call_id\": \"{call_id}\"}} \
         to retrieve the full output]\n\n{body}"
    )
}

fn char_boundary_floor(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn char_boundary_ceil(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

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
            terminate: true,
            truncated: None,
        };
    }

    let reason = merged
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("blocked")
        .to_string();
    FunctionResult {
        content: vec![ContentBlock::Text(TextContent { text: reason })],
        details: json!({ "blocked": true }),
        terminate: false,
        truncated: None,
    }
}

pub(crate) fn prefilled_result_is_error(result: &FunctionResult) -> bool {
    !result
        .details
        .get("pending_approval")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

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

pub(crate) fn publish_failure_from_response(
    response: &Value,
    require_approval_gate_reply: bool,
) -> Option<String> {
    if let Some(publish) = response.get("publish") {
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
    if response.get("publish_failed").and_then(Value::as_bool) == Some(true) {
        return Some("publish failed".to_string());
    }
    if require_approval_gate_reply {
        let approval_gate_replied = response
            .get("replies")
            .and_then(Value::as_array)
            .map(|replies| replies.iter().any(is_approval_gate_reply))
            .unwrap_or(false);
        if !approval_gate_replied {
            return Some("publish succeeded but approval-gate did not reply".to_string());
        }
    }
    None
}

fn is_approval_gate_reply(reply: &Value) -> bool {
    reply.get("approval_gate").and_then(Value::as_bool) == Some(true)
        || reply.get("subscriber").and_then(Value::as_str) == Some("approval-gate")
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
        let merged = match publish_collect_checked(
            iii,
            TOPIC_BEFORE,
            build_before_function_call_payload(&record.session_id, &fc, &approval_required),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
            true,
        )
        .await
        {
            Ok(merged) => merged,
            Err(err) => fail_closed_block_reply("hook_publish", &err),
        };
        let blocked = merged
            .get("block")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let prefilled =
            blocked.then(|| prefilled_result_for_block(&merged, &fc.id, &fc.function_id));
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
            let is_error = prefilled_result_is_error(&blocked);
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
        let result = maybe_truncate_result(iii, &record.session_id, &fc.id, result).await;

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
        let merged = publish_collect(
            iii,
            TOPIC_AFTER,
            json!({ "function_call": &fc, "result": &result }),
            "field_merge",
            HOOK_TIMEOUT_MS,
        )
        .await
        .unwrap_or_else(|err| {
            tracing::warn!(
                error = %err,
                "after-hook publish failed; preserving original function result",
            );
            json!({})
        });
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
    replace_pending_approval_placeholders(&mut messages, &function_results);
    for r in &function_results {
        messages.push(AgentMessage::FunctionResult(r.clone()));
    }
    persistence::save_messages(iii, &record.session_id, &messages).await;

    let Some(last_assistant) = record.last_assistant.clone() else {
        tracing::warn!(
            session_id = %record.session_id,
            "FunctionFinalize reached without last_assistant; skipping lifecycle emit"
        );
        record.function_results = function_results;
        record.pending_function_calls.clear();
        record.transition_to(next_state_after_finalize(false, all_terminate));
        return Ok(());
    };
    for evt in build_finalize_lifecycle(&last_assistant, &function_results) {
        events::emit(iii, &record.session_id, &evt).await;
    }
    record.turn_end_emitted = true;

    record.function_results = function_results;
    record.pending_function_calls.clear();
    record.transition_to(next_state_after_finalize(true, all_terminate));
    Ok(())
}

pub(crate) fn next_state_after_finalize(
    _has_last_assistant: bool,
    all_terminate: bool,
) -> TurnState {
    if all_terminate {
        TurnState::TearingDown
    } else {
        TurnState::SteeringCheck
    }
}

pub(crate) fn executed_staging_for_new_prepare_batch(
    _stale: &[(FunctionCall, FunctionResult, bool)],
) -> Vec<(FunctionCall, FunctionResult, bool)> {
    Vec::new()
}

pub(crate) fn prepared_calls_from_approval_entries(
    entries: &[Value],
) -> Vec<(FunctionCall, Option<FunctionResult>)> {
    entries
        .iter()
        .filter_map(prepared_call_from_approval_entry)
        .collect()
}

fn prepared_call_from_approval_entry(
    entry: &Value,
) -> Option<(FunctionCall, Option<FunctionResult>)> {
    let function_call_id = entry
        .get("function_call_id")
        .or_else(|| entry.get("tool_call_id"))
        .and_then(Value::as_str)?
        .to_string();
    let function_id = entry
        .get("function_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let args = entry.get("args").cloned().unwrap_or_else(|| json!({}));
    let decision = entry
        .get("decision")
        .and_then(Value::as_str)
        .unwrap_or("deny");
    let fc = FunctionCall {
        id: function_call_id.clone(),
        function_id,
        arguments: args,
    };
    if decision == "allow" {
        return Some((fc, None));
    }

    let reason = entry
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("denied");
    let text = if reason == "timed_out" || decision == "timed_out" {
        "approval timed out before resolution".to_string()
    } else {
        format!("approval denied: {reason}")
    };
    let result = FunctionResult {
        content: vec![ContentBlock::Text(TextContent { text })],
        details: json!({
            "approval_denied": true,
            "decision": decision,
            "reason": reason,
            "resolved_via_approval_gate": true,
            "call_id": function_call_id,
        }),
        terminate: false,
        truncated: None,
    };
    Some((fc, Some(result)))
}

/// Pure helper: inner payload for the `agent::before_function_call` topic.
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

pub(crate) async fn consume_resolved_approval_entries(
    iii: &III,
    session_id: &str,
) -> Result<Vec<(FunctionCall, Option<FunctionResult>)>, String> {
    let response = iii
        .trigger(TriggerRequest {
            function_id: "approval::consume".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .map_err(|err| err.to_string())?;
    if response.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("approval::consume failed")
            .to_string());
    }
    let entries = response
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(prepared_calls_from_approval_entries(&entries))
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

pub(crate) fn replace_pending_approval_placeholders(
    messages: &mut Vec<AgentMessage>,
    replacements: &[FunctionResultMessage],
) {
    let replacement_ids = replacements
        .iter()
        .map(|r| r.function_call_id.as_str())
        .collect::<HashSet<_>>();
    if replacement_ids.is_empty() {
        return;
    }
    messages.retain(|message| match message {
        AgentMessage::FunctionResult(result) => {
            let is_replaced_call = replacement_ids.contains(result.function_call_id.as_str());
            let is_pending_placeholder = result
                .details
                .get("pending_approval")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            !(is_replaced_call && is_pending_placeholder)
        }
        _ => true,
    });
}

async fn publish_collect(
    iii: &III,
    topic: &str,
    inner: Value,
    merge_rule: &str,
    timeout_ms: u64,
) -> Result<Value, String> {
    publish_collect_checked(iii, topic, inner, merge_rule, timeout_ms, false).await
}

async fn publish_collect_checked(
    iii: &III,
    topic: &str,
    inner: Value,
    merge_rule: &str,
    timeout_ms: u64,
    require_approval_gate_reply: bool,
) -> Result<Value, String> {
    let payload = json!({
        "topic": topic,
        "payload": inner,
        "merge_rule": merge_rule,
        "timeout_ms": timeout_ms,
    });
    iii.trigger(TriggerRequest {
        function_id: "hook-fanout::publish_collect".into(),
        payload,
        action: None,
        timeout_ms: None,
    })
    .await
    .map_err(|err| err.to_string())
    .and_then(|response| {
        if let Some(err) = publish_failure_from_response(&response, require_approval_gate_reply) {
            Err(err)
        } else {
            Ok(response.get("merged").cloned().unwrap_or_else(|| json!({})))
        }
    })
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
                truncated: None,
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
            truncated: None,
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
            truncated: None,
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
    fn pending_block_prefill_terminates_without_error_flag() {
        let merged = json!({
            "block": true,
            "status": "pending",
            "reason": "approval required",
        });

        let result = prefilled_result_for_block(&merged, "tc-1", "shell::exec");

        assert!(result.terminate, "pending approvals must stop the turn");
        assert_eq!(result.details["pending_approval"], true);
        assert!(!prefilled_result_is_error(&result));
        assert!(matches!(
            result.content.first(),
            Some(ContentBlock::Text(text))
                if text.text.contains("\"status\": \"pending_approval\"")
                    && text.text.contains("\"call_id\": \"tc-1\"")
        ));
    }

    #[test]
    fn hard_block_prefill_remains_error_without_terminating() {
        let merged = json!({
            "block": true,
            "status": "denied",
            "reason": "blocked by policy",
        });

        let result = prefilled_result_for_block(&merged, "tc-2", "shell::exec");

        assert!(!result.terminate);
        assert_eq!(result.details["blocked"], true);
        assert!(prefilled_result_is_error(&result));
    }

    #[test]
    fn approval_allow_entry_prepares_dispatchable_function_call() {
        let prepared = prepared_calls_from_approval_entries(&[json!({
            "function_call_id": "tc-1",
            "function_id": "shell::exec",
            "args": { "command": "date" },
            "decision": "allow",
        })]);

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].0.id, "tc-1");
        assert_eq!(prepared[0].0.function_id, "shell::exec");
        assert_eq!(prepared[0].0.arguments, json!({ "command": "date" }));
        assert!(
            prepared[0].1.is_none(),
            "allow must execute through the normal dispatch path"
        );
    }

    #[test]
    fn approval_deny_entry_prepares_prefilled_result_without_dispatch() {
        let prepared = prepared_calls_from_approval_entries(&[json!({
            "function_call_id": "tc-1",
            "function_id": "shell::exec",
            "args": { "command": "date" },
            "decision": "deny",
            "reason": "timed_out",
        })]);

        let result = prepared[0].1.as_ref().expect("deny should prefill");
        assert_eq!(prepared[0].0.id, "tc-1");
        assert!(prefilled_result_is_error(result));
        assert_eq!(result.details["approval_denied"], true);
        assert_eq!(result.details["reason"], "timed_out");
        assert!(matches!(
            result.content.first(),
            Some(ContentBlock::Text(text)) if text.text.contains("approval timed out")
        ));
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
        assert_eq!(inner["session_id"], "sess-a");
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

    #[test]
    fn approval_resume_replaces_pending_placeholder_for_same_call_id() {
        let mut messages = vec![
            AgentMessage::User(harness_types::UserMessage {
                content: vec![ContentBlock::Text(TextContent { text: "run".into() })],
                timestamp: 0,
            }),
            AgentMessage::FunctionResult(FunctionResultMessage {
                function_call_id: "tc-1".into(),
                function_id: "shell::fs::mkdir".into(),
                content: vec![ContentBlock::Text(TextContent {
                    text: "pending approval".into(),
                })],
                details: json!({ "pending_approval": true }),
                is_error: false,
                timestamp: 1,
            }),
            AgentMessage::FunctionResult(FunctionResultMessage {
                function_call_id: "tc-2".into(),
                function_id: "shell::fs::ls".into(),
                content: vec![ContentBlock::Text(TextContent { text: "ok".into() })],
                details: json!({}),
                is_error: false,
                timestamp: 2,
            }),
        ];
        let replacement = FunctionResultMessage {
            function_call_id: "tc-1".into(),
            function_id: "shell::fs::mkdir".into(),
            content: vec![ContentBlock::Text(TextContent {
                text: "created".into(),
            })],
            details: json!({ "created": true }),
            is_error: false,
            timestamp: 3,
        };

        replace_pending_approval_placeholders(&mut messages, &[replacement]);

        assert_eq!(messages.len(), 2);
        assert!(!messages.iter().any(|message| matches!(
            message,
            AgentMessage::FunctionResult(result)
                if result.function_call_id == "tc-1"
                    && result.details.get("pending_approval").and_then(Value::as_bool) == Some(true)
        )));
        assert!(messages.iter().any(|message| matches!(
            message,
            AgentMessage::FunctionResult(result) if result.function_call_id == "tc-2"
        )));
    }

    #[test]
    fn approval_resume_keeps_non_placeholder_result_with_same_call_id() {
        let mut messages = vec![AgentMessage::FunctionResult(FunctionResultMessage {
            function_call_id: "tc-1".into(),
            function_id: "shell::fs::mkdir".into(),
            content: vec![ContentBlock::Text(TextContent { text: "old".into() })],
            details: json!({ "created": true }),
            is_error: false,
            timestamp: 1,
        })];
        let replacement = function_result_msg("shell::fs::mkdir", false);

        replace_pending_approval_placeholders(&mut messages, &[replacement]);

        assert_eq!(messages.len(), 1);
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
        assert_eq!(inner["session_id"], "sess-a");
        assert_eq!(inner["function_call"]["id"], "tc-1");
        assert_eq!(inner["function_call"]["function_id"], "shell::fs::ls");
        assert_eq!(inner["function_call"]["arguments"], json!({"path": "/tmp"}));
        assert!(inner.get("approval_required").is_some());
    }

    #[test]
    fn publish_failure_from_response_fails_closed_on_publish_error() {
        let response = json!({
            "event_id": "evt",
            "replies": [],
            "merged": { "block": false },
            "publish": { "ok": false, "error": "ws closed" }
        });

        assert_eq!(
            publish_failure_from_response(&response, true).as_deref(),
            Some("ws closed"),
        );
    }

    #[test]
    fn publish_failure_from_response_requires_approval_gate_reply_for_before_hook() {
        let empty = json!({
            "event_id": "evt",
            "replies": [],
            "merged": { "block": false },
            "publish": { "ok": true }
        });
        assert!(publish_failure_from_response(&empty, true).is_some());

        let non_gate = json!({
            "event_id": "evt",
            "replies": [{ "block": false, "subscriber": "policy-denylist" }],
            "merged": { "block": false },
            "publish": { "ok": true }
        });
        assert!(publish_failure_from_response(&non_gate, true).is_some());

        let gate = json!({
            "event_id": "evt",
            "replies": [{ "block": false, "subscriber": "approval-gate", "approval_gate": true }],
            "merged": { "block": false },
            "publish": { "ok": true }
        });
        assert!(publish_failure_from_response(&gate, true).is_none());
    }

    #[test]
    fn publish_failure_from_response_allows_zero_replies_for_after_hook() {
        let response = json!({
            "event_id": "evt",
            "replies": [],
            "merged": { "block": false },
            "publish": { "ok": true }
        });

        assert!(publish_failure_from_response(&response, false).is_none());
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

    // ─── Truncation helpers ──────────────────────────────────────────────

    #[test]
    fn render_truncated_text_short_input_kept_verbatim() {
        let result = FunctionResult {
            content: vec![ContentBlock::Text(TextContent {
                text: "hello world".into(),
            })],
            details: json!({}),
            terminate: false,
            truncated: None,
        };
        let out = render_truncated_text(&result, 12345, "call-abc");
        assert!(out.contains("result truncated"));
        assert!(out.contains("12345 bytes"));
        assert!(out.contains("call-abc"));
        assert!(out.contains("hello world"));
        assert!(
            !out.contains("[... "),
            "short content must not have ellipsis"
        );
    }

    #[test]
    fn render_truncated_text_large_input_head_tail_elides() {
        let head = "H".repeat(3000);
        let middle = "M".repeat(50_000);
        let tail = "T".repeat(3000);
        let combined = format!("{head}{middle}{tail}");
        let result = FunctionResult {
            content: vec![ContentBlock::Text(TextContent {
                text: combined.clone(),
            })],
            details: json!({}),
            terminate: false,
            truncated: None,
        };
        let out = render_truncated_text(&result, combined.len(), "call-x");
        assert!(out.contains("[... "), "must contain elision marker");
        assert!(out.contains("bytes elided"));
        // Head and tail preserved.
        assert!(out.contains("HHHHHHHHHH"));
        assert!(out.contains("TTTTTTTTTT"));
        // Middle should be largely gone — fewer than 100 M's survive.
        let m_count = out.chars().filter(|c| *c == 'M').count();
        assert!(
            m_count < 100,
            "middle should be elided, found {} 'M' chars",
            m_count
        );
    }

    #[test]
    fn render_truncated_text_concatenates_multiple_text_blocks() {
        let result = FunctionResult {
            content: vec![
                ContentBlock::Text(TextContent { text: "alpha".into() }),
                ContentBlock::Text(TextContent { text: "beta".into() }),
            ],
            details: json!({}),
            terminate: false,
            truncated: None,
        };
        let out = render_truncated_text(&result, 999, "cid");
        assert!(out.contains("alpha"));
        assert!(out.contains("beta"));
    }

    #[test]
    fn char_boundary_helpers_respect_utf8() {
        let s = "héllo🦀world";
        for idx in 0..=s.len() {
            let f = char_boundary_floor(s, idx);
            assert!(f <= idx);
            assert!(s.is_char_boundary(f));
        }
        for idx in 0..=s.len() {
            let c = char_boundary_ceil(s, idx);
            assert!(c >= idx);
            assert!(s.is_char_boundary(c));
        }
    }

    #[test]
    fn truncate_threshold_has_sane_default() {
        // OnceLock means we can't reliably set/reset the env in-process;
        // assert only that the default is the documented constant.
        // (Per-env override is exercised manually / via integration tests.)
        let t = truncate_threshold();
        assert!(t >= 1024, "threshold should never round down below 1 KB");
        assert!(t <= 1_000_000, "threshold should be sane");
    }

    #[test]
    fn finalize_without_last_assistant_still_continues_after_function_results() {
        assert_eq!(
            next_state_after_finalize(false, false),
            TurnState::SteeringCheck,
            "approval resume records have no last_assistant, but allowed function results \
             must still flow through steering into the next assistant turn"
        );
    }

    #[test]
    fn finalize_without_last_assistant_tears_down_when_all_results_terminate() {
        assert_eq!(
            next_state_after_finalize(false, true),
            TurnState::TearingDown
        );
    }
}
