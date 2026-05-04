//! Reactive overflow watcher.
//!
//! Subscribes to the `agent::events` stream via a `stream` trigger.
//! When an event carries `error_kind == "context_overflow"`, republishes it
//! to the `agent::transform_context` pubsub topic so downstream
//! `transform_context` subscribers can produce a compacted message tail.
//!
//! Stateless — no per-session bookkeeping. Drives a different code path
//! than the [`crate::compactor`] module, which is proactive (threshold-
//! triggered) rather than reactive (post-error-triggered).

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, Trigger, TriggerAction,
    TriggerRequest, III,
};
use serde_json::{json, Value};

const FN_ID: &str = "context_compaction::watcher";
const STREAM: &str = "agent::events";
const TRANSFORM_TOPIC: &str = "agent::transform_context";

/// Decide whether an `AgentEvent` (the wire form) signals a context-overflow
/// condition that warrants a `transform_context` publication.
pub fn payload_signals_overflow(data: &Value) -> bool {
    let kind = data.get("type").and_then(Value::as_str);
    let Some(kind) = kind else { return false };
    let message = match kind {
        "message_end" | "message_start" | "message_update" | "turn_end" => data.get("message"),
        _ => None,
    };
    let Some(message) = message else { return false };
    matches!(
        message.get("error_kind").and_then(Value::as_str),
        Some("context_overflow")
    )
}

/// Handle for the watcher's registered function + trigger.
pub struct WatcherHandle {
    function: Option<FunctionRef>,
    trigger: Option<Trigger>,
}

impl WatcherHandle {
    pub fn unregister_all(mut self) {
        if let Some(t) = self.trigger.take() {
            t.unregister();
        }
        if let Some(f) = self.function.take() {
            f.unregister();
        }
    }
}

/// Register the watcher's function + stream trigger.
pub fn register(iii: &III) -> Result<WatcherHandle, IIIError> {
    let iii_for_handler = iii.clone();
    let function = iii.register_function((
        RegisterFunctionMessage::with_id(FN_ID.into()).with_description(
            "Republish context_overflow events on agent::events to agent::transform_context."
                .into(),
        ),
        move |payload: Value| {
            let iii = iii_for_handler.clone();
            async move {
                let data = payload.get("data").cloned().unwrap_or(Value::Null);
                if !payload_signals_overflow(&data) {
                    return Ok(json!({ "ok": true, "republished": false }));
                }
                iii.trigger(TriggerRequest {
                    function_id: "publish".into(),
                    payload: json!({
                        "topic": TRANSFORM_TOPIC,
                        "data": {
                            "reason": "context_overflow",
                            "source_event": data,
                        },
                    }),
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                })
                .await
                .map_err(|e| IIIError::Handler(e.to_string()))?;
                Ok(json!({ "ok": true, "republished": true }))
            }
        },
    ));
    let trigger = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "stream".into(),
        function_id: FN_ID.into(),
        config: json!({ "stream_name": STREAM }),
        metadata: None,
    })?;
    Ok(WatcherHandle {
        function: Some(function),
        trigger: Some(trigger),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_types::{
        AgentEvent, AgentMessage, AssistantMessage, ContentBlock, ErrorKind, StopReason,
        TextContent, UserMessage,
    };

    fn assistant_with_overflow() -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            content: vec![ContentBlock::Text(TextContent {
                text: "ran out".into(),
            })],
            stop_reason: StopReason::Error,
            error_message: Some("context window exceeded".into()),
            error_kind: Some(ErrorKind::ContextOverflow),
            usage: None,
            model: "claude-opus-4-7".into(),
            provider: "anthropic".into(),
            timestamp: 0,
        })
    }

    #[test]
    fn payload_signals_overflow_message_end() {
        let event = AgentEvent::MessageEnd {
            message: assistant_with_overflow(),
        };
        let data = serde_json::to_value(&event).unwrap();
        assert!(payload_signals_overflow(&data));
    }

    #[test]
    fn payload_signals_overflow_turn_end() {
        let event = AgentEvent::TurnEnd {
            message: assistant_with_overflow(),
            tool_results: Vec::new(),
        };
        let data = serde_json::to_value(&event).unwrap();
        assert!(payload_signals_overflow(&data));
    }

    #[test]
    fn payload_does_not_signal_when_no_error_kind() {
        let event = AgentEvent::MessageEnd {
            message: AgentMessage::User(UserMessage {
                content: vec![ContentBlock::Text(TextContent { text: "hi".into() })],
                timestamp: 0,
            }),
        };
        let data = serde_json::to_value(&event).unwrap();
        assert!(!payload_signals_overflow(&data));
    }

    #[test]
    fn payload_does_not_signal_for_unrelated_events() {
        let data = serde_json::to_value(&AgentEvent::AgentStart).unwrap();
        assert!(!payload_signals_overflow(&data));
        let data = serde_json::json!({ "type": "tool_execution_start" });
        assert!(!payload_signals_overflow(&data));
    }
}
