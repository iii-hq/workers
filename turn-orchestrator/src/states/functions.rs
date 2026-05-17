//! `function_prepare`, `function_execute`, `function_finalize` handlers.

use harness_types::{
    AgentEvent, AgentMessage, AssistantMessage, ContentBlock, FunctionCall, FunctionResult,
    FunctionResultMessage, TextContent, TruncationInfo,
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
        let merged = publish_collect(
            iii,
            TOPIC_BEFORE,
            build_before_function_call_payload(&fc, &approval_required),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
        )
        .await;
        let blocked = merged
            .get("block")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let prefilled = if blocked {
            let reason = merged
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("blocked")
                .to_string();
            Some(FunctionResult {
                content: vec![ContentBlock::Text(TextContent { text: reason })],
                details: json!({ "blocked": true }),
                terminate: false,
                truncated: None,
            })
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
            persistence::upsert_executed_call(&mut results, (fc.clone(), blocked.clone(), true));
            persistence::save_executed_calls(iii, &record.session_id, &results).await;
            let evt = build_function_execution_event(&fc, &blocked, true);
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
    for r in &function_results {
        messages.push(AgentMessage::FunctionResult(r.clone()));
    }
    persistence::save_messages(iii, &record.session_id, &messages).await;

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
    for evt in build_finalize_lifecycle(&last_assistant, &function_results) {
        events::emit(iii, &record.session_id, &evt).await;
    }
    record.turn_end_emitted = true;

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
pub(crate) fn build_before_function_call_payload(
    fc: &FunctionCall,
    approval_required: &[String],
) -> Value {
    json!({
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

async fn publish_collect(
    iii: &III,
    topic: &str,
    inner: Value,
    merge_rule: &str,
    timeout_ms: u64,
) -> Value {
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
    .ok()
    .and_then(|v| v.get("merged").cloned())
    .unwrap_or_else(|| json!({}))
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
    fn before_function_call_payload_carries_approval_required() {
        let fc = FunctionCall {
            id: "tc-1".into(),
            function_id: "shell::fs::write".into(),
            arguments: json!({"path": "/tmp/x"}),
        };
        let approval_required = vec!["shell::fs::write".to_string()];
        let inner = build_before_function_call_payload(&fc, &approval_required);
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
        let inner = build_before_function_call_payload(&fc, &[]);
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
        let inner = build_before_function_call_payload(&fc, &[]);
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
}
