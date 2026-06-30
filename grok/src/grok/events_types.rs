//! Serde mirror of the streaming-json events `grok --print --output-format
//! streaming-json` writes to stdout, one JSON object per line.
//!
//! PROVISIONAL: the Grok Build CLI streaming-json schema is not published, so
//! the typed shapes below are a thread/item event model to be validated against
//! a real capture (`grok -p "hi" --output-format streaming-json`). Parsing is
//! intentionally lenient — unknown event/item types fall through to `Unknown`
//! and are logged + skipped rather than failing the turn, and every line is
//! always forwarded verbatim onto `grok::events`, so an out-of-date typed model
//! only reduces the normalized `agent::events` frames, never drops raw data.

use serde::Deserialize;
use serde_json::Value;

/// Top-level event, externally tagged on `type`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ThreadEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted(ThreadStartedEvent),
    #[serde(rename = "turn.started")]
    TurnStarted(Value),
    #[serde(rename = "turn.completed")]
    TurnCompleted(TurnCompletedEvent),
    #[serde(rename = "turn.failed")]
    TurnFailed(TurnFailedEvent),
    #[serde(rename = "item.started")]
    ItemStarted(ItemEvent),
    #[serde(rename = "item.updated")]
    ItemUpdated(ItemEvent),
    #[serde(rename = "item.completed")]
    ItemCompleted(ItemEvent),
    #[serde(rename = "error")]
    Error(ThreadErrorEvent),
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThreadStartedEvent {
    pub thread_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TurnCompletedEvent {
    pub usage: Usage,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub cached_input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub reasoning_output_tokens: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TurnFailedEvent {
    pub error: ThreadErrorEvent,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThreadErrorEvent {
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ItemEvent {
    pub item: ThreadItem,
}

/// `{ id, ...details }` — details are flattened and tagged on `type`.
#[derive(Debug, Clone, Deserialize)]
pub struct ThreadItem {
    pub id: String,
    #[serde(flatten)]
    pub details: ThreadItemDetails,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThreadItemDetails {
    AgentMessage {
        text: String,
    },
    Reasoning {
        text: String,
    },
    CommandExecution(CommandExecutionItem),
    FileChange(FileChangeItem),
    McpToolCall(McpToolCallItem),
    WebSearch(WebSearchItem),
    TodoList(Value),
    Error {
        message: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandExecutionItem {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub aggregated_output: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub status: ItemStatus,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileChangeItem {
    #[serde(default)]
    pub changes: Value,
    #[serde(default)]
    pub status: ItemStatus,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpToolCallItem {
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub error: Option<McpError>,
    #[serde(default)]
    pub status: ItemStatus,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpError {
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebSearchItem {
    #[serde(default)]
    pub query: String,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
    Declined,
}

impl ItemStatus {
    pub fn is_failure(&self) -> bool {
        matches!(self, ItemStatus::Failed)
    }
}
