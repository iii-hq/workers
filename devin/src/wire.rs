//! Wire types emitted onto `agent::events` (an AgentEvent subset) and the
//! persisted session record. Mirrors the harness AgentEvent shape so the
//! console and acp worker render devin::run turns like any other agent worker.
//!
//! The `devin` CLI streams plain text, not token usage, so unlike some agent
//! workers there is no `Usage` type here.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Working,
    Done,
    Error,
}

/// A local record mapping an iii session_id to the Devin cloud session the
/// `devin` CLI opened for it, so status/history survive restarts. `cwd` is the
/// directory the CLI ran in; `model` is retained for AgentEvent shape parity
/// and is usually empty for Devin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub devin_session_id: Option<String>,
    pub cwd: String,
    pub model: String,
    pub status: Status,
    pub turns: i64,
    pub updated_at_ms: u64,
}

/// One block of assistant content on the stream.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Thinking { text: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct AssistantMessage {
    pub role: &'static str,
    pub content: Vec<ContentBlock>,
    pub stop_reason: String,
    pub model: String,
    pub provider: &'static str,
    pub timestamp: u64,
}

pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn assistant_message(content: Vec<ContentBlock>, model: &str, stop_reason: &str) -> Value {
    serde_json::to_value(AssistantMessage {
        role: "assistant",
        content,
        stop_reason: stop_reason.to_string(),
        model: model.to_string(),
        provider: "devin",
        timestamp: now_ms(),
    })
    .unwrap_or(Value::Null)
}
