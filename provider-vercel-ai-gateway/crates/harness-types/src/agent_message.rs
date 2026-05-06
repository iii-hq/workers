use serde::{Deserialize, Serialize};

use crate::content::ContentBlock;
use crate::stream_event::{ErrorKind, StopReason, Usage};
use crate::thinking::ThinkingLevel;
use crate::tool::AgentTool;

/// Transcript message. Superset of LLM message types plus app-defined custom entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum AgentMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
    Custom(CustomMessage),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMessage {
    pub content: Vec<ContentBlock>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<ErrorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    pub model: String,
    pub provider: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: Vec<ContentBlock>,
    pub details: serde_json::Value,
    pub is_error: bool,
    pub timestamp: i64,
}

/// App-defined custom transcript entries. Filtered out during `convert_to_llm`
/// unless the converter explicitly maps them to a user message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomMessage {
    pub custom_type: String,
    pub content: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(default)]
    pub details: serde_json::Value,
    pub timestamp: i64,
}

/// Snapshot of session-level state passed into an LLM call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentContext {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    #[serde(default)]
    pub tools: Vec<AgentTool>,
}

/// Persisted session state. Lives at `agent::session/<id>/state` on iii state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSessionState {
    pub session_id: String,
    pub model: String,
    #[serde(default)]
    pub thinking_level: ThinkingLevel,
    #[serde(default)]
    pub abort_signal: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_roundtrip() {
        let m = AgentMessage::User(UserMessage {
            content: vec![ContentBlock::Text(crate::content::TextContent {
                text: "hi".into(),
            })],
            timestamp: 1,
        });
        let json = serde_json::to_string(&m).unwrap();
        let back: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn role_tag_distinguishes_variants() {
        let json = r#"{"role":"user","content":[],"timestamp":0}"#;
        let m: AgentMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(m, AgentMessage::User(_)));
    }
}
