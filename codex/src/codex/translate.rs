//! Pure translation of the Codex JSONL event stream into the AgentEvent frames
//! emitted on `agent::events`, plus the running turn state (thread id, usage,
//! result text, stop reason). Kept free of I/O so the full per-turn sequence is
//! unit-testable without a live engine — the stream loop in `mod.rs` only does
//! the reads/writes around `step`.

use std::collections::HashSet;

use serde_json::{json, Value};

use super::events_types::{ThreadEvent, ThreadItemDetails};
use crate::map;
use crate::wire::{assistant_message, ContentBlock, Usage};

#[derive(Default)]
pub struct TurnState {
    started: HashSet<String>,
    pub thread_id: Option<String>,
    pub usage: Option<Usage>,
    pub result_text: String,
    pub stop_reason: String,
    pub is_error: bool,
    /// Model id stamped onto message_complete frames.
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

/// Advance the turn by one event, returning the `agent::events` frames to emit
/// (in order). Updates `state` in place.
pub fn step(state: &mut TurnState, event: ThreadEvent) -> Vec<Value> {
    match event {
        ThreadEvent::ThreadStarted(e) => {
            state.thread_id = Some(e.thread_id);
            vec![]
        }
        ThreadEvent::ItemStarted(ev) | ThreadEvent::ItemUpdated(ev) => {
            if map::is_exec_item(&ev.item.details) && state.started.insert(ev.item.id.clone()) {
                vec![exec_start(&ev.item.id, &ev.item.details)]
            } else {
                vec![]
            }
        }
        ThreadEvent::ItemCompleted(ev) => item_completed(state, &ev.item.id, &ev.item.details),
        ThreadEvent::TurnCompleted(e) => {
            state.usage = Some(map::map_usage(&e.usage));
            vec![]
        }
        ThreadEvent::TurnFailed(e) => {
            state.is_error = true;
            state.stop_reason = "error".to_string();
            state.result_text = e.error.message;
            vec![]
        }
        ThreadEvent::Error(e) => {
            state.is_error = true;
            state.stop_reason = "error".to_string();
            state.result_text = e.message;
            vec![]
        }
        ThreadEvent::TurnStarted(_) | ThreadEvent::Unknown => vec![],
    }
}

fn exec_start(id: &str, details: &ThreadItemDetails) -> Value {
    json!({
        "type": "function_execution_start",
        "function_call_id": id,
        "function_id": map::function_id(details),
        "args": map::args_for(details),
    })
}

fn item_completed(state: &mut TurnState, id: &str, details: &ThreadItemDetails) -> Vec<Value> {
    match details {
        ThreadItemDetails::AgentMessage { text } | ThreadItemDetails::Reasoning { text } => {
            let is_agent = matches!(details, ThreadItemDetails::AgentMessage { .. });
            let block = if is_agent {
                ContentBlock::Text { text: text.clone() }
            } else {
                ContentBlock::Thinking { text: text.clone() }
            };
            if is_agent {
                state.result_text = text.clone();
            }
            let msg = assistant_message(vec![block], &state.model, None, "end");
            vec![json!({ "type": "message_complete", "message": msg })]
        }
        d if map::is_exec_item(d) => {
            let mut frames = Vec::new();
            if state.started.insert(id.to_string()) {
                frames.push(exec_start(id, d));
            }
            frames.push(json!({
                "type": "function_execution_end",
                "function_call_id": id,
                "function_id": map::function_id(d),
                "result": { "content": map::result_content(d), "details": null },
                "is_error": map::is_error_item(d),
                "duration_ms": 0,
            }));
            frames
        }
        _ => vec![],
    }
}
