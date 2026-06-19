//! Wire and Telegram API types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type JsonMap = serde_json::Map<String, Value>;

// ---------------------------------------------------------------------------
// Telegram Bot API (subset)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TelegramUpdate {
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<TelegramMessage>,
    #[serde(default)]
    pub callback_query: Option<TelegramCallbackQuery>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TelegramMessage {
    pub message_id: i64,
    pub chat: TelegramChat,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub photo: Option<Vec<TelegramPhotoSize>>,
    #[serde(default)]
    pub document: Option<TelegramDocument>,
    #[serde(default)]
    pub voice: Option<TelegramVoice>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TelegramPhotoSize {
    pub file_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TelegramDocument {
    pub file_id: String,
    #[serde(default)]
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TelegramVoice {
    pub file_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TelegramChat {
    pub id: i64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TelegramCallbackQuery {
    pub id: String,
    #[serde(default)]
    pub from: Option<TelegramUser>,
    pub message: Option<TelegramMessage>,
    #[serde(default)]
    pub data: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TelegramUser {
    pub id: i64,
}

// ---------------------------------------------------------------------------
// HTTP trigger envelope
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct HttpTriggerRequest {
    #[serde(default)]
    pub body: Value,
    #[serde(default)]
    pub headers: Option<JsonMap>,
}

// ---------------------------------------------------------------------------
// Session / harness event payloads (subset)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MessageAddedEvent {
    pub session_id: String,
    pub entry_id: String,
    /// Parent entry in the session chain (authoritative append order).
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub message: Option<AgentMessage>,
    /// Append time in ms; used as the per-entry ordering key.
    #[serde(default)]
    pub timestamp: i64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MessageUpdatedEvent {
    pub session_id: String,
    pub entry_id: String,
    pub message: AgentMessage,
    pub revision: u64,
    /// Mutation time in ms; refines the per-entry ordering key (min wins).
    #[serde(default)]
    pub timestamp: i64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StatusChangedEvent {
    pub session_id: String,
    pub status: String,
    #[serde(default)]
    pub status_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TurnCompletedEvent {
    pub session_id: String,
    pub turn_id: String,
    pub status: String,
    #[serde(default)]
    pub result_error: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AgentMessage {
    pub role: String,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        text: String,
    },
    FunctionCall {
        id: String,
        function_id: String,
        arguments: Value,
    },
    #[serde(other)]
    Other,
}

// ---------------------------------------------------------------------------
// Approval gate payloads (subset)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PendingApprovalRecord {
    pub session_id: String,
    pub function_call_id: String,
    pub function_id: String,
    #[serde(default)]
    pub arguments_excerpt: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PendingResolvedEvent {
    pub session_id: String,
    pub function_call_id: String,
    pub outcome: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ResolveDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalCallbackData {
    pub session_id: String,
    pub function_call_id: String,
    pub function_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatFsm {
    Idle,
    AwaitingModel,
}

impl ChatFsm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::AwaitingModel => "awaiting_model",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "awaiting_model" => Self::AwaitingModel,
            _ => Self::Idle,
        }
    }
}
