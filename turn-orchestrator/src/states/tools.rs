//! `tool_prepare`, `tool_execute`, `tool_finalize` handlers.

use harness_types::{
    AgentEvent, AgentMessage, AssistantMessage, ContentBlock, TextContent, ToolCall, ToolResult,
    ToolResultMessage,
};
use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;

use crate::events;
use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};

const TOPIC_BEFORE: &str = "agent::before_tool_call";
const TOPIC_AFTER: &str = "agent::after_tool_call";
const HOOK_TIMEOUT_MS: u64 = 10_000;

pub async fn handle_prepare(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    record.tool_results.clear();
    let calls = record.pending_tool_calls.clone();

    // run_request is immutable for a session; loading it here on every retry of
    // ToolPrepare is wasteful but correct. Cache on TurnStateRecord if hot.
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

    let mut prepared: Vec<(ToolCall, Option<ToolResult>)> = Vec::with_capacity(calls.len());
    for tc in calls {
        let merged = publish_collect(
            iii,
            TOPIC_BEFORE,
            build_before_tool_call_payload(&tc, &approval_required),
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
            Some(ToolResult {
                content: vec![ContentBlock::Text(TextContent { text: reason })],
                details: json!({ "blocked": true }),
                terminate: false,
            })
        } else {
            None
        };
        prepared.push((tc, prefilled));
    }

    persistence::save_record(iii, record).await;
    let executed = executed_staging_for_new_prepare_batch(&[]);
    persistence::save_executed_calls(iii, &record.session_id, &executed).await;
    persistence::save_prepared_calls(iii, &record.session_id, &prepared).await;

    record.transition_to(TurnState::ToolExecute);
    Ok(())
}

pub async fn handle_execute(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    let prepared = persistence::load_prepared_calls(iii, &record.session_id).await;
    let mut results = persistence::load_executed_calls(iii, &record.session_id).await;
    for (tc, prefilled) in prepared {
        events::emit(
            iii,
            &record.session_id,
            &AgentEvent::ToolExecutionStart {
                tool_call_id: tc.id.clone(),
                tool_name: tc.name.clone(),
                args: tc.arguments.clone(),
            },
        )
        .await;
        if let Some(blocked) = prefilled {
            persistence::upsert_executed_call(&mut results, (tc.clone(), blocked.clone(), true));
            persistence::save_executed_calls(iii, &record.session_id, &results).await;
            let evt = build_tool_execution_event(&tc, &blocked, true);
            events::emit(iii, &record.session_id, &evt).await;
            continue;
        }
        if let Some((_, recorded, recorded_is_error)) =
            persistence::find_executed_call(&results, &tc.id).cloned()
        {
            let evt = build_tool_execution_event(&tc, &recorded, recorded_is_error);
            events::emit(iii, &record.session_id, &evt).await;
            continue;
        }
        // Tool handlers expect the model's arguments at the top level (e.g.
        // `args.path` for shell::filesystem::ls). Augment with session_id and
        // tool metadata so handlers can correlate calls with the active session.
        let mut payload = match tc.arguments.clone() {
            Value::Object(o) => Value::Object(o),
            other => json!({ "arguments": other }),
        };
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("session_id".into(), json!(record.session_id));
            obj.insert("tool_call_id".into(), json!(tc.id));
            obj.insert("tool_name".into(), json!(tc.name));
            obj.insert(
                "tool_call".into(),
                json!({
                    "id": tc.id.clone(),
                    "name": tc.name.clone(),
                    "arguments": tc.arguments.clone(),
                }),
            );
        }
        let response = iii
            .trigger(TriggerRequest {
                function_id: tc.name.clone(),
                payload,
                action: None,
                timeout_ms: None,
            })
            .await;
        let (result, is_error) = match response {
            Ok(v) => (decode_tool_result(v), false),
            Err(e) => (
                ToolResult {
                    content: vec![ContentBlock::Text(TextContent {
                        text: format!("tool '{}' failed: {e}", tc.name),
                    })],
                    details: json!({}),
                    terminate: false,
                },
                true,
            ),
        };
        persistence::upsert_executed_call(&mut results, (tc.clone(), result.clone(), is_error));
        persistence::save_executed_calls(iii, &record.session_id, &results).await;
        let evt = build_tool_execution_event(&tc, &result, is_error);
        events::emit(iii, &record.session_id, &evt).await;
    }
    record.transition_to(TurnState::ToolFinalize);
    Ok(())
}

pub async fn handle_finalize(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    let executed = persistence::load_executed_calls(iii, &record.session_id).await;
    let mut tool_results: Vec<ToolResultMessage> = Vec::with_capacity(executed.len());
    let mut all_terminate = !executed.is_empty();
    for (tc, mut result, is_error) in executed {
        let merged = publish_collect(
            iii,
            TOPIC_AFTER,
            json!({ "tool_call": tc, "result": result }),
            "field_merge",
            HOOK_TIMEOUT_MS,
        )
        .await;
        if let Ok(after) = serde_json::from_value::<ToolResult>(merged.clone()) {
            result = after;
        }
        if !result.terminate {
            all_terminate = false;
        }
        tool_results.push(ToolResultMessage {
            tool_call_id: tc.id,
            tool_name: tc.name,
            content: result.content,
            details: result.details,
            is_error,
            timestamp: chrono::Utc::now().timestamp_millis(),
        });
    }

    let mut messages = persistence::load_messages(iii, &record.session_id).await;
    for r in &tool_results {
        messages.push(AgentMessage::ToolResult(r.clone()));
    }
    persistence::save_messages(iii, &record.session_id, &messages).await;

    let last_assistant = record
        .last_assistant
        .clone()
        .expect("tools state requires last_assistant; only assistant_finished transitions in");
    for evt in build_finalize_lifecycle(&last_assistant, &tool_results) {
        events::emit(iii, &record.session_id, &evt).await;
    }
    record.turn_end_emitted = true;

    record.tool_results = tool_results;
    record.pending_tool_calls.clear();
    if all_terminate {
        record.transition_to(TurnState::TearingDown);
    } else {
        record.transition_to(TurnState::SteeringCheck);
    }
    Ok(())
}

pub(crate) fn executed_staging_for_new_prepare_batch(
    _stale: &[(ToolCall, ToolResult, bool)],
) -> Vec<(ToolCall, ToolResult, bool)> {
    Vec::new()
}

/// Pure helper: build the inner payload for the `agent::before_tool_call`
/// topic. Subscribers (policy-denylist, approval-gate) read this shape.
pub(crate) fn build_before_tool_call_payload(tc: &ToolCall, approval_required: &[String]) -> Value {
    json!({
        "tool_call": tc,
        "approval_required": approval_required,
    })
}

/// Pure helper: build the [`AgentEvent::ToolExecutionEnd`] for one tool.
pub(crate) fn build_tool_execution_event(
    tc: &ToolCall,
    result: &ToolResult,
    is_error: bool,
) -> AgentEvent {
    AgentEvent::ToolExecutionEnd {
        tool_call_id: tc.id.clone(),
        tool_name: tc.name.clone(),
        is_error,
        result: result.clone(),
    }
}

/// Pure helper: build the lifecycle events emitted at the end of a
/// tool-bearing turn: `MessageStart`/`MessageEnd` per tool result, then
/// one `TurnEnd` carrying the assistant message and all tool results.
pub(crate) fn build_finalize_lifecycle(
    assistant: &AssistantMessage,
    tool_results: &[ToolResultMessage],
) -> Vec<AgentEvent> {
    let mut events = Vec::with_capacity(tool_results.len() * 2 + 1);
    for r in tool_results {
        let m = AgentMessage::ToolResult(r.clone());
        events.push(AgentEvent::MessageStart { message: m.clone() });
        events.push(AgentEvent::MessageEnd { message: m });
    }
    events.push(AgentEvent::TurnEnd {
        message: AgentMessage::Assistant(assistant.clone()),
        tool_results: tool_results.to_vec(),
    });
    events
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
        function_id: "hooks::publish_collect".into(),
        payload,
        action: None,
        timeout_ms: None,
    })
    .await
    .ok()
    .and_then(|v| v.get("merged").cloned())
    .unwrap_or_else(|| json!({}))
}

fn decode_tool_result(value: Value) -> ToolResult {
    serde_json::from_value::<ToolResult>(value.clone()).unwrap_or_else(|_| {
        let content = value
            .get("content")
            .and_then(|c| serde_json::from_value::<Vec<ContentBlock>>(c.clone()).ok())
            .unwrap_or_else(|| {
                vec![ContentBlock::Text(TextContent {
                    text: value.to_string(),
                })]
            });
        let details = value.get("details").cloned().unwrap_or_else(|| json!({}));
        let terminate = value
            .get("terminate")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        ToolResult {
            content,
            details,
            terminate,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_types::{AgentEvent, AssistantMessage, ContentBlock, TextContent, ToolCall};

    fn assistant_with_tool_call(name: &str) -> AssistantMessage {
        AssistantMessage {
            content: vec![ContentBlock::ToolCall {
                id: "tc-1".into(),
                name: name.into(),
                arguments: json!({}),
            }],
            stop_reason: harness_types::StopReason::Tool,
            error_message: None,
            error_kind: None,
            usage: None,
            model: "m".into(),
            provider: "p".into(),
            timestamp: 0,
        }
    }

    fn tool_result_msg(name: &str, is_error: bool) -> ToolResultMessage {
        ToolResultMessage {
            tool_call_id: "tc-1".into(),
            tool_name: name.into(),
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
            ToolCall {
                id: "tc-1".into(),
                name: "read".into(),
                arguments: json!({}),
            },
            ToolResult {
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
    fn build_tool_execution_event_carries_tool_name_and_error_flag() {
        let tc = ToolCall {
            id: "tc-1".into(),
            name: "read".into(),
            arguments: json!({"path": "/tmp/x"}),
        };
        let result = ToolResult {
            content: vec![ContentBlock::Text(TextContent { text: "ok".into() })],
            details: json!({}),
            terminate: false,
        };
        let evt = build_tool_execution_event(&tc, &result, false);
        match evt {
            AgentEvent::ToolExecutionEnd {
                tool_name,
                is_error,
                ..
            } => {
                assert_eq!(tool_name, "read");
                assert!(!is_error);
            }
            other => panic!("expected ToolExecutionEnd, got {other:?}"),
        }
    }

    #[test]
    fn build_tool_execution_event_marks_blocked_tool_as_error() {
        let tc = ToolCall {
            id: "tc-2".into(),
            name: "bash".into(),
            arguments: json!({"command": "rm -rf /"}),
        };
        let blocked = ToolResult {
            content: vec![ContentBlock::Text(TextContent {
                text: "blocked by policy".into(),
            })],
            details: json!({"blocked": true}),
            terminate: false,
        };
        let evt = build_tool_execution_event(&tc, &blocked, true);
        match evt {
            AgentEvent::ToolExecutionEnd {
                tool_name,
                is_error,
                result,
                ..
            } => {
                assert_eq!(tool_name, "bash");
                assert!(is_error);
                assert!(matches!(
                    result.content.first(),
                    Some(ContentBlock::Text(t)) if t.text == "blocked by policy"
                ));
            }
            other => panic!("expected ToolExecutionEnd, got {other:?}"),
        }
    }

    #[test]
    fn before_tool_call_payload_carries_approval_required() {
        let tc = ToolCall {
            id: "tc-1".into(),
            name: "shell::filesystem::write".into(),
            arguments: json!({"path": "/tmp/x"}),
        };
        let approval_required = vec!["shell::filesystem::write".to_string()];
        let inner = build_before_tool_call_payload(&tc, &approval_required);
        assert_eq!(inner["tool_call"]["id"], "tc-1");
        assert_eq!(
            inner["approval_required"],
            json!(["shell::filesystem::write"]),
        );
    }

    #[test]
    fn before_tool_call_payload_has_empty_approval_required_when_none_configured() {
        let tc = ToolCall {
            id: "tc-1".into(),
            name: "shell::filesystem::ls".into(),
            arguments: json!({}),
        };
        let inner = build_before_tool_call_payload(&tc, &[]);
        assert_eq!(inner["approval_required"], json!([]));
    }

    #[test]
    fn build_finalize_lifecycle_emits_pair_per_tool_then_turn_end() {
        let asst = assistant_with_tool_call("read");
        let results = vec![
            tool_result_msg("read", false),
            tool_result_msg("write", false),
        ];
        let events = build_finalize_lifecycle(&asst, &results);
        assert_eq!(events.len(), 5);
        assert!(matches!(&events[0], AgentEvent::MessageStart { .. }));
        assert!(matches!(events.last(), Some(AgentEvent::TurnEnd { .. })));
    }
}
