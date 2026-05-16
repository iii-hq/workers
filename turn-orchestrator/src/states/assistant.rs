//! `awaiting_assistant`, `assistant_streaming`, `assistant_finished` handlers.

use harness_types::{
    AgentEvent, AgentMessage, AssistantMessage, ContentBlock, FunctionCall, StopReason,
    TextContent, UserMessage,
};
use iii_sdk::{TriggerRequest, III};
use serde_json::{json, Value};

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
                function_results: Vec::new(),
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

/// Drain resolved approvals from the gate via `approval::consume` and
/// convert them into stitched user messages plus an optional omission
/// summary. Returns `(stitched_msgs, summary_msg_or_none)`.
///
/// `approval::consume` is atomic: it returns Done rows and deletes them
/// in the same call. No turn_id stamping (delivered_in_turn_id is gone);
/// rows beyond the gate's cap stay in state and surface on the next turn,
/// counted via `omitted`.
///
/// Network failures are logged and swallowed: a transient consume failure
/// must not block the turn. On failure the caller proceeds with an empty
/// stitch — entries will be retried on the next turn.
pub(crate) async fn consume_approval_stitch(
    iii: &III,
    session_id: &str,
) -> (Vec<AgentMessage>, Option<String>) {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "approval::consume".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    let resp = match resp {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "approval::consume failed; skipping stitch this turn");
            return (Vec::new(), None);
        }
    };
    let entries = resp
        .get("entries")
        .and_then(|e| e.as_array().cloned())
        .unwrap_or_default();
    let omitted = resp.get("omitted").and_then(Value::as_u64).unwrap_or(0);
    if entries.is_empty() && omitted == 0 {
        return (Vec::new(), None);
    }
    let stitched = crate::states::approval_stitching::stitch_entries(&entries);
    let msgs = stitched_strings_to_messages(stitched, chrono::Utc::now().timestamp_millis());
    let summary = crate::states::approval_stitching::omission_summary_message(omitted);
    (msgs, summary)
}

/// Wrap each stitched approval-info string into an `AgentMessage::User`.
///
/// **Why `User` and not `Assistant`:** the Anthropic wire contract requires
/// the messages array to end with `role: "user"`. Wrapping these as
/// `Assistant` made the request end with an assistant message and the API
/// rejected the turn with "This model does not support assistant message
/// prefill. The conversation must end with a user message." The text payload
/// is already prefixed with `[approval-gate]` so the model knows it didn't
/// author the line — same convention as `tool_result` blocks (which also
/// land on `role: "user"` via `to_wire_messages`).
pub(crate) fn stitched_strings_to_messages(
    stitched: Vec<String>,
    timestamp: i64,
) -> Vec<AgentMessage> {
    stitched
        .into_iter()
        .map(|text| {
            AgentMessage::User(UserMessage {
                content: vec![ContentBlock::Text(TextContent { text })],
                timestamp,
            })
        })
        .collect()
}

/// Append stitched approval messages onto persisted conversation history.
/// Pure — caller is responsible for saving the resulting history with
/// `persistence::save_messages`.
pub(crate) fn merge_stitched_into_history(
    history: &mut Vec<AgentMessage>,
    stitched: Vec<AgentMessage>,
) {
    if stitched.is_empty() {
        return;
    }
    history.extend(stitched);
}

/// Append a one-line summary user message (e.g. omission warning) onto
/// history. No-op when `summary` is `None`.
pub(crate) fn append_summary_message(
    history: &mut Vec<AgentMessage>,
    summary: Option<String>,
    timestamp: i64,
) {
    let Some(text) = summary else {
        return;
    };
    history.push(AgentMessage::User(UserMessage {
        content: vec![ContentBlock::Text(TextContent { text })],
        timestamp,
    }));
}

pub async fn handle_streaming(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    let request = persistence::load_run_request(iii, &record.session_id).await;
    let mut messages = persistence::load_messages(iii, &record.session_id).await;
    let (stitch_msgs, summary) =
        consume_approval_stitch(iii, &record.session_id).await;
    let stitched_nonempty = !stitch_msgs.is_empty() || summary.is_some();
    merge_stitched_into_history(&mut messages, stitch_msgs);
    append_summary_message(
        &mut messages,
        summary,
        chrono::Utc::now().timestamp_millis(),
    );
    // Persist BEFORE the LLM call. If the call fails, next turn resumes
    // from a history that already contains the stitched approvals; no
    // re-fetch from approval-gate, no accumulation.
    if stitched_nonempty {
        persistence::save_messages(iii, &record.session_id, &messages).await;
    }
    let schemas = persistence::load_function_schemas(iii, &record.session_id).await;

    let payload = json!({
        "session_id": record.session_id,
        "provider": request.get("provider").cloned().unwrap_or_else(|| json!("")),
        "model": request.get("model").cloned().unwrap_or_else(|| json!("")),
        "system_prompt": request.get("system_prompt").cloned().unwrap_or_else(|| json!("")),
        "messages": messages,
        "tools": schemas,
    });
    let response = iii
        .trigger(TriggerRequest {
            function_id: "router::stream_assistant".into(),
            payload,
            action: None,
            timeout_ms: Some(300_000),
        })
        .await
        .map_err(|e| anyhow::anyhow!("router::stream_assistant failed: {e}"))?;
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
                function_results: Vec::new(),
            },
        )
        .await;
        record.turn_end_emitted = true;
        record.transition_to(TurnState::TearingDown);
        return Ok(());
    }

    let calls = extract_function_calls(&assistant);
    if calls.is_empty() {
        record.transition_to(TurnState::SteeringCheck);
    } else {
        record.pending_function_calls = calls;
        record.transition_to(TurnState::FunctionPrepare);
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

fn extract_function_calls(assistant: &AssistantMessage) -> Vec<FunctionCall> {
    assistant
        .content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::FunctionCall {
                id,
                function_id,
                arguments,
            } => Some(FunctionCall {
                id: id.clone(),
                function_id: function_id.clone(),
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
            content: vec![ContentBlock::FunctionCall {
                id: "x".into(),
                function_id: "read".into(),
                arguments: json!({}),
            }],
            stop_reason: StopReason::FunctionCall,
            error_message: None,
            error_kind: None,
            usage: None,
            model: "test".into(),
            provider: "test".into(),
            timestamp: 0,
        }
    }

    #[test]
    fn extract_function_calls_collects_function_blocks_only() {
        assert!(extract_function_calls(&assistant_text()).is_empty());
        let calls = extract_function_calls(&assistant_tool());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "read");
    }

    /// Regression: stitched approval entries must wrap as `User`, not
    /// `Assistant`. Wrapping as `Assistant` produced a wire payload ending
    /// with `role: "assistant"`, which Anthropic rejects with the prefill
    /// error ("The conversation must end with a user message").
    #[test]
    fn stitched_strings_wrap_as_user_messages_not_assistant() {
        let out = stitched_strings_to_messages(
            vec!["[approval-gate] Earlier call_id c1 ...".into()],
            1234,
        );
        assert_eq!(out.len(), 1);
        match &out[0] {
            AgentMessage::User(u) => {
                assert_eq!(u.timestamp, 1234);
                match u.content.first() {
                    Some(ContentBlock::Text(t)) => {
                        assert!(t.text.starts_with("[approval-gate]"));
                    }
                    other => panic!("expected Text content block, got {other:?}"),
                }
            }
            other => panic!("stitched messages must be AgentMessage::User, got {other:?}"),
        }
    }

    #[test]
    fn stitched_strings_to_messages_preserves_order_and_count() {
        let out = stitched_strings_to_messages(
            vec!["a".into(), "b".into(), "c".into()],
            0,
        );
        assert_eq!(out.len(), 3);
        let texts: Vec<String> = out
            .iter()
            .map(|m| match m {
                AgentMessage::User(u) => match &u.content[0] {
                    ContentBlock::Text(t) => t.text.clone(),
                    _ => panic!("expected text"),
                },
                _ => panic!("expected user"),
            })
            .collect();
        assert_eq!(texts, vec!["a", "b", "c"]);
    }

    #[test]
    fn stitched_strings_to_messages_empty_input_returns_empty() {
        assert!(stitched_strings_to_messages(vec![], 0).is_empty());
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

    /// Regression for the unbounded-redelivery bug: stitched messages must
    /// be persisted into conversation history BEFORE the LLM call. If the
    /// LLM errors after this point, the next turn loads them from history
    /// rather than re-fetching from approval-gate and re-accumulating.
    #[test]
    fn stitched_messages_must_be_pushed_into_persisted_history() {
        let stitched = stitched_strings_to_messages(
            vec!["[approval-gate] Earlier call_id c1 ...".into()],
            1234,
        );
        let mut history: Vec<AgentMessage> = Vec::new();
        merge_stitched_into_history(&mut history, stitched);
        assert_eq!(history.len(), 1);
        match &history[0] {
            AgentMessage::User(u) => match &u.content[0] {
                ContentBlock::Text(t) => assert!(t.text.starts_with("[approval-gate]")),
                _ => panic!("expected text content"),
            },
            _ => panic!("expected user message"),
        }
    }

    #[test]
    fn merge_stitched_into_history_with_empty_stitched_is_noop() {
        let mut history: Vec<AgentMessage> = vec![AgentMessage::User(UserMessage {
            content: vec![ContentBlock::Text(TextContent { text: "hi".into() })],
            timestamp: 1,
        })];
        merge_stitched_into_history(&mut history, Vec::new());
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn merge_stitched_into_history_appends_summary_when_provided() {
        let stitched = stitched_strings_to_messages(vec!["[approval-gate] entry 1".into()], 1);
        let mut history: Vec<AgentMessage> = Vec::new();
        merge_stitched_into_history(&mut history, stitched);
        append_summary_message(&mut history, Some("[approval-gate] 5 omitted".into()), 2);
        assert_eq!(history.len(), 2);
        match &history[1] {
            AgentMessage::User(u) => match &u.content[0] {
                ContentBlock::Text(t) => assert!(t.text.contains("5 omitted")),
                _ => panic!("expected text"),
            },
            _ => panic!("expected user message"),
        }
    }

    #[test]
    fn append_summary_message_none_is_noop() {
        let mut history: Vec<AgentMessage> = Vec::new();
        append_summary_message(&mut history, None, 1);
        assert!(history.is_empty());
    }
}
