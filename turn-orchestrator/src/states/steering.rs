//! `steering_check` handler: drain steering queue, follow-up queue, abort flag.

use harness_types::{AgentEvent, AgentMessage, AssistantMessage, ErrorKind, StopReason};
use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;

use crate::events;
use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};

pub async fn handle(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    if abort_set(iii, &record.session_id).await {
        // Abort: build a legacy-shaped aborted message, persist it onto
        // the transcript, then emit TurnEnd carrying it. Mirror of
        // `harness-runtime/src/loop_state.rs:139-148` and
        // `loop_state.rs:321-332`.
        let aborted = aborted_message();
        let mut messages = persistence::load_messages(iii, &record.session_id).await;
        messages.push(AgentMessage::Assistant(aborted.clone()));
        persistence::save_messages(iii, &record.session_id, &messages).await;
        record.last_assistant = Some(aborted.clone());
        if !record.turn_end_emitted {
            events::emit(
                iii,
                &record.session_id,
                &AgentEvent::TurnEnd {
                    message: AgentMessage::Assistant(aborted),
                    tool_results: Vec::new(),
                },
            )
            .await;
            record.turn_end_emitted = true;
        }
        record.transition_to(TurnState::TearingDown);
        return Ok(());
    }

    let steering = drain_queue(iii, "steering", &record.session_id).await;
    if !steering.is_empty() {
        emit_turn_end_once(iii, record).await;
        let mut messages = persistence::load_messages(iii, &record.session_id).await;
        messages.extend(steering);
        persistence::save_messages(iii, &record.session_id, &messages).await;
        record.transition_to(TurnState::AwaitingAssistant);
        return Ok(());
    }

    let followup = drain_queue(iii, "followup", &record.session_id).await;
    if !followup.is_empty() {
        emit_turn_end_once(iii, record).await;
        let mut messages = persistence::load_messages(iii, &record.session_id).await;
        messages.extend(followup);
        persistence::save_messages(iii, &record.session_id, &messages).await;
        record.transition_to(TurnState::AwaitingAssistant);
        return Ok(());
    }

    emit_turn_end_once(iii, record).await;
    record.transition_to(TurnState::TearingDown);
    Ok(())
}

/// Emit `TurnEnd` only when the current turn hasn't already emitted one
/// (`tools::handle_finalize` and `assistant::handle_awaiting`/`handle_finished`'s
/// terminating branches set `turn_end_emitted = true`). For no-tool turns
/// reaching `steering_check` from `handle_finished` directly, this is the
/// single emission point. Mirrors legacy `loop_state.rs:194-200`.
async fn emit_turn_end_once(iii: &III, record: &mut TurnStateRecord) {
    if record.turn_end_emitted {
        return;
    }
    let message = AgentMessage::Assistant(record.last_assistant.clone().unwrap_or_else(|| {
        AssistantMessage {
            content: Vec::new(),
            stop_reason: StopReason::End,
            error_message: None,
            error_kind: None,
            usage: None,
            model: String::new(),
            provider: String::new(),
            timestamp: chrono::Utc::now().timestamp_millis(),
        }
    }));
    events::emit(
        iii,
        &record.session_id,
        &AgentEvent::TurnEnd {
            message,
            tool_results: Vec::new(),
        },
    )
    .await;
    record.turn_end_emitted = true;
}

/// Legacy-shaped aborted assistant message — mirror of
/// `harness-runtime/src/loop_state.rs:321-332`. Kept private to the
/// steering handler since this is the only place an abort produces a
/// synthetic message in the durable path.
fn aborted_message() -> AssistantMessage {
    AssistantMessage {
        content: Vec::new(),
        stop_reason: StopReason::Aborted,
        error_message: Some("aborted".into()),
        error_kind: Some(ErrorKind::Transient),
        usage: None,
        model: "harness".into(),
        provider: "harness".into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    }
}

async fn abort_set(iii: &III, session_id: &str) -> bool {
    iii.trigger(TriggerRequest {
        function_id: state_flag::IS_SET_ID.into(),
        payload: json!({ "name": "abort", "session_id": session_id }),
        action: None,
        timeout_ms: None,
    })
    .await
    .ok()
    .and_then(|v| v.get("value").and_then(Value::as_bool))
    .unwrap_or(false)
}

async fn drain_queue(iii: &III, name: &str, session_id: &str) -> Vec<AgentMessage> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: durable_queue::DRAIN_ID.into(),
            payload: json!({ "name": name, "session_id": session_id }),
            action: None,
            timeout_ms: None,
        })
        .await;
    let value = match resp {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    value
        .get("items")
        .cloned()
        .map(serde_json::from_value::<Vec<AgentMessage>>)
        .transpose()
        .ok()
        .flatten()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aborted_message_matches_legacy_shape() {
        let m = aborted_message();
        assert_eq!(m.stop_reason, StopReason::Aborted);
        assert_eq!(m.model, "harness");
        assert_eq!(m.provider, "harness");
        assert!(matches!(m.error_kind, Some(ErrorKind::Transient)));
        assert_eq!(m.error_message.as_deref(), Some("aborted"));
        assert!(m.content.is_empty());
    }
}
