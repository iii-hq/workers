//! Pure translation of the Grok streaming-json event stream into the AgentEvent
//! frames emitted on `agent::events`, plus the running turn state (session id,
//! result text, stop reason). Kept free of I/O so the full per-turn sequence is
//! unit-testable without a live engine — the stream loop in `mod.rs` only does
//! the reads/writes around `step`.

use serde_json::{json, Value};

use super::events_types::GrokEvent;
use crate::wire::{assistant_message, ContentBlock};

#[derive(Default)]
pub struct TurnState {
    /// Grok's own session id, captured from the terminal `end` event; the key
    /// passed to `--resume` on the next turn.
    pub thread_id: Option<String>,
    pub result_text: String,
    /// Accumulated `thought` deltas (reasoning models); rendered as a thinking
    /// block, kept out of `result_text` (the returned answer).
    pub thought_text: String,
    pub stop_reason: String,
    pub is_error: bool,
    /// Model id stamped onto the message_complete frame.
    pub model: String,
}

impl TurnState {
    pub fn new(model: String) -> Self {
        Self {
            stop_reason: "end".to_string(),
            model,
            ..Default::default()
        }
    }
}

/// Map a Grok `stopReason` onto the AgentEvent stop-reason vocabulary.
fn map_stop_reason(raw: &str) -> String {
    match raw {
        "EndTurn" | "" => "end".to_string(),
        "MaxTokens" => "max_tokens".to_string(),
        "Aborted" | "Cancelled" => "aborted".to_string(),
        other => other.to_string(),
    }
}

/// Advance the turn by one event, returning the `agent::events` frames to emit
/// (in order). Updates `state` in place.
pub fn step(state: &mut TurnState, event: GrokEvent) -> Vec<Value> {
    match event {
        GrokEvent::Text { data } => {
            state.result_text.push_str(&data);
            vec![]
        }
        GrokEvent::Thought { data } => {
            state.thought_text.push_str(&data);
            vec![]
        }
        GrokEvent::End(e) => {
            if !e.session_id.is_empty() {
                state.thread_id = Some(e.session_id);
            }
            state.stop_reason = map_stop_reason(&e.stop_reason);
            let mut content = Vec::new();
            if !state.thought_text.is_empty() {
                content.push(ContentBlock::Thinking {
                    text: state.thought_text.clone(),
                });
            }
            content.push(ContentBlock::Text {
                text: state.result_text.clone(),
            });
            let msg = assistant_message(content, &state.model, &state.stop_reason);
            vec![json!({ "type": "message_complete", "message": msg })]
        }
        GrokEvent::Error { message } => {
            state.is_error = true;
            state.stop_reason = "error".to_string();
            state.result_text = message;
            vec![]
        }
        GrokEvent::Unknown => vec![],
    }
}
