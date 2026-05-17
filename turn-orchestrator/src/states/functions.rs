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
pub(crate) fn prefilled_result_for_block(merged: &Value, call_id: &str, function_id: &str) -> FunctionResult {
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
    let reason = merged
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("blocked")
        .to_string();
    FunctionResult {
        content: vec![ContentBlock::Text(TextContent { text: reason })],
        details: json!({ "blocked": true }),
        terminate: false,
    }
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
            build_before_function_call_payload(&record.session_id, &fc, &approval_required),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
        )
        .await;
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
        assert!(fr.terminate, "pending_approval result must terminate the turn");
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
    }
}
