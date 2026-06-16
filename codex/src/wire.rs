//! Wire types emitted onto `agent::events` (translated AgentEvent subset) and
//! the persisted session record. Mirrors the harness AgentEvent shape so the
//! console and acp worker render Codex turns like any other agent worker.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub reasoning_tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Working,
    Done,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub codex_thread_id: Option<String>,
    pub cwd: String,
    pub model: String,
    pub status: Status,
    pub turns: i64,
    pub usage: Option<Usage>,
    pub updated_at_ms: u64,
}

/// One block of assistant/tool content on the translated stream.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Thinking { text: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct AssistantMessage {
    pub role: &'static str, // "assistant"
    pub content: Vec<ContentBlock>,
    pub stop_reason: String,
    pub model: String,
    pub provider: &'static str, // "codex"
    pub usage: Option<Usage>,
    pub timestamp: u64,
}

pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn assistant_message(
    content: Vec<ContentBlock>,
    model: &str,
    usage: Option<Usage>,
    stop_reason: &str,
) -> Value {
    serde_json::to_value(AssistantMessage {
        role: "assistant",
        content,
        stop_reason: stop_reason.to_string(),
        model: model.to_string(),
        provider: "codex",
        usage,
        timestamp: now_ms(),
    })
    .unwrap_or(Value::Null)
}
