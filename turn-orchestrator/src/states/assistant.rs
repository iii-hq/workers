//! `awaiting_assistant`, `assistant_streaming`, `assistant_finished` handlers.

use harness_types::{
    AgentEvent, AgentMessage, AssistantMessage, ContentBlock, StopReason, ToolCall,
};
use iii_sdk::{TriggerRequest, III};
use serde_json::json;

use crate::events;
use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};

pub async fn handle_awaiting(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    if record
        .max_turns
        .map_or(false, |cap| record.turn_count >= cap)
    {
        // Synthetic max-turns assistant message, mirroring loop_state.rs behavior.
        let exhausted = AssistantMessage {
            content: vec![ContentBlock::Text(harness_types::TextContent {
                text: format!(
                    "loop stopped: max_turns ({}) reached",
                    record.max_turns.unwrap_or(0)
                ),
            })],
            stop_reason: StopReason::End,
            error_message: None,
            error_kind: None,
            usage: None,
            model: String::new(),
            provider: String::new(),
            timestamp: chrono::Utc::now().timestamp_millis(),
        };
        let exhausted_msg = AgentMessage::Assistant(exhausted.clone());
        // Emit the synthetic message + a TurnEnd so consumers see a clean
        // termination instead of a silent transition into TearingDown.
        events::emit(
            iii,
            &record.session_id,
            &AgentEvent::MessageStart {
                message: exhausted_msg.clone(),
            },
        )
        .await;
        events::emit(
            iii,
            &record.session_id,
            &AgentEvent::MessageEnd {
                message: exhausted_msg.clone(),
            },
        )
        .await;
        events::emit(
            iii,
            &record.session_id,
            &AgentEvent::TurnEnd {
                message: exhausted_msg,
                tool_results: Vec::new(),
            },
        )
        .await;
        record.turn_end_emitted = true;
        record.last_assistant = Some(exhausted.clone());
        let mut messages = persistence::load_messages(iii, &record.session_id).await;
        messages.push(AgentMessage::Assistant(exhausted));
        persistence::save_messages(iii, &record.session_id, &messages).await;
        record.transition_to(TurnState::TearingDown);
        return Ok(());
    }
    record.turn_count += 1;
    record.turn_end_emitted = false;
    events::emit(iii, &record.session_id, &AgentEvent::TurnStart).await;
    record.transition_to(TurnState::AssistantStreaming);
    Ok(())
}

pub async fn handle_streaming(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    let request = persistence::load_run_request(iii, &record.session_id).await;
    let messages = persistence::load_messages(iii, &record.session_id).await;
    let tools = persistence::load_tool_schemas(iii, &record.session_id).await;

    let payload = json!({
        "session_id": record.session_id,
        "provider": request.get("provider").cloned().unwrap_or_else(|| json!("")),
        "model": request.get("model").cloned().unwrap_or_else(|| json!("")),
        "system_prompt": request.get("system_prompt").cloned().unwrap_or_else(|| json!("")),
        "messages": messages,
        "tools": tools,
    });
    let response = iii
        .trigger(TriggerRequest {
            function_id: "agent::stream_assistant".into(),
            payload,
            action: None,
            timeout_ms: Some(300_000),
        })
        .await
        .map_err(|e| anyhow::anyhow!("agent::stream_assistant failed: {e}"))?;
    let assistant: AssistantMessage =
        serde_json::from_value(response).map_err(|e| anyhow::anyhow!("decode assistant: {e}"))?;
    record.last_assistant = Some(assistant);
    record.transition_to(TurnState::AssistantFinished);
    Ok(())
}

pub async fn handle_finished(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    let assistant = record
        .last_assistant
        .clone()
        .ok_or_else(|| anyhow::anyhow!("assistant_finished without last_assistant"))?;

    for evt in assistant_lifecycle_events(&assistant) {
        events::emit(iii, &record.session_id, &evt).await;
    }

    let mut messages = persistence::load_messages(iii, &record.session_id).await;
    messages.push(AgentMessage::Assistant(assistant.clone()));
    persistence::save_messages(iii, &record.session_id, &messages).await;

    if matches!(
        assistant.stop_reason,
        StopReason::Error | StopReason::Aborted
    ) {
        // Error / aborted assistant closes the turn here. Emit TurnEnd
        // with no tool results so consumers know the turn ended.
        events::emit(
            iii,
            &record.session_id,
            &AgentEvent::TurnEnd {
                message: AgentMessage::Assistant(assistant),
                tool_results: Vec::new(),
            },
        )
        .await;
        record.turn_end_emitted = true;
        record.transition_to(TurnState::TearingDown);
        return Ok(());
    }

    let tool_calls = extract_tool_calls(&assistant);
    if tool_calls.is_empty() {
        record.transition_to(TurnState::SteeringCheck);
    } else {
        record.pending_tool_calls = tool_calls;
        record.transition_to(TurnState::ToolPrepare);
    }
    Ok(())
}

/// Pure helper: events the orchestrator emits when an assistant message
/// is decoded. Lifted out for unit-testability.
pub(crate) fn assistant_lifecycle_events(assistant: &AssistantMessage) -> Vec<AgentEvent> {
    let msg = AgentMessage::Assistant(assistant.clone());
    vec![
        AgentEvent::MessageStart {
            message: msg.clone(),
        },
        AgentEvent::MessageEnd { message: msg },
    ]
}

fn extract_tool_calls(assistant: &AssistantMessage) -> Vec<ToolCall> {
    assistant
        .content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
            } => Some(ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_types::TextContent;

    fn assistant_text() -> AssistantMessage {
        AssistantMessage {
            content: vec![ContentBlock::Text(TextContent { text: "ok".into() })],
            stop_reason: StopReason::End,
            error_message: None,
            error_kind: None,
            usage: None,
            model: "test".into(),
            provider: "test".into(),
            timestamp: 0,
        }
    }

    fn assistant_tool() -> AssistantMessage {
        AssistantMessage {
            content: vec![ContentBlock::ToolCall {
                id: "x".into(),
                name: "read".into(),
                arguments: json!({}),
            }],
            stop_reason: StopReason::Tool,
            error_message: None,
            error_kind: None,
            usage: None,
            model: "test".into(),
            provider: "test".into(),
            timestamp: 0,
        }
    }

    #[test]
    fn extract_tool_calls_collects_tool_blocks_only() {
        assert!(extract_tool_calls(&assistant_text()).is_empty());
        let calls = extract_tool_calls(&assistant_tool());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read");
    }

    #[test]
    fn assistant_lifecycle_events_orders_message_start_before_end() {
        let asst = assistant_text();
        let events = assistant_lifecycle_events(&asst);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            harness_types::AgentEvent::MessageStart { .. }
        ));
        assert!(matches!(
            &events[1],
            harness_types::AgentEvent::MessageEnd { .. }
        ));
    }
}
