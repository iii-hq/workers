//! Wire types: Slack events/interactions and engine session/approval events.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Slack Events API
// ---------------------------------------------------------------------------

/// Outer envelope Slack POSTs to the events URL.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackEnvelope {
    UrlVerification {
        challenge: String,
    },
    EventCallback(Box<EventCallback>),
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventCallback {
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub event_id: Option<String>,
    pub event: SlackEvent,
}

/// The inner events the bridge cares about. Unknown events deserialize to `Other`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackEvent {
    AppMention(MessageEvent),
    Message(MessageEvent),
    AssistantThreadStarted {
        assistant_thread: AssistantThread,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageEvent {
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub ts: Option<String>,
    #[serde(default)]
    pub thread_ts: Option<String>,
    /// `im`, `channel`, `group`, `mpim` — present on `message` events.
    #[serde(default)]
    pub channel_type: Option<String>,
    /// Set on edits, joins, bot messages, etc. We ignore messages with a subtype.
    #[serde(default)]
    pub subtype: Option<String>,
    /// Present when the message was posted by a bot (including ourselves).
    #[serde(default)]
    pub bot_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantThread {
    pub channel_id: String,
    pub thread_ts: String,
    #[serde(default)]
    pub user_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Slack interactivity (block_actions)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct InteractionPayload {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub user: Option<InteractionUser>,
    #[serde(default)]
    pub actions: Vec<BlockAction>,
    #[serde(default)]
    pub channel: Option<IdObject>,
    #[serde(default)]
    pub message: Option<InteractionMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InteractionUser {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IdObject {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InteractionMessage {
    #[serde(default)]
    pub ts: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockAction {
    pub action_id: String,
    #[serde(default)]
    pub value: Option<String>,
}

/// Persisted (in iii-state) context for an approval button click.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalCallback {
    pub session_id: String,
    pub function_call_id: String,
    pub function_id: String,
    pub channel: String,
    pub message_ts: String,
}

// ---------------------------------------------------------------------------
// Engine session / harness / approval events (subset)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MessageAddedEvent {
    pub session_id: String,
    pub entry_id: String,
    #[serde(default)]
    pub message: Option<AgentMessage>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MessageUpdatedEvent {
    pub session_id: String,
    pub entry_id: String,
    pub message: AgentMessage,
    pub revision: u64,
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
        function_id: String,
    },
    #[serde(other)]
    Other,
}

impl AgentMessage {
    /// Concatenate the assistant's visible text blocks.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for block in &self.content {
            if let ContentBlock::Text { text } = block {
                out.push_str(text);
            }
        }
        out
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PendingApprovalRecord {
    pub session_id: String,
    pub function_call_id: String,
    pub function_id: String,
    #[serde(default)]
    pub arguments_excerpt: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PendingResolvedEvent {
    pub session_id: String,
    pub function_call_id: String,
    #[serde(default)]
    pub outcome: Option<String>,
}
