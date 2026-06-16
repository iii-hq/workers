use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::content::ContentBlock;
use crate::types::events::{ErrorKind, StopReason, Usage};

/// Single-variant role tags: exact-match on deserialize, correct wire string on
/// serialize, and they let `AgentMessage` be an untagged union.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum UserRoleTag {
    #[serde(rename = "user")]
    User,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum AssistantRoleTag {
    #[serde(rename = "assistant")]
    Assistant,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum FunctionResultRoleTag {
    #[serde(rename = "function_result")]
    FunctionResult,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum CustomRoleTag {
    #[serde(rename = "custom")]
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UserMessage {
    pub role: UserRoleTag,
    pub content: Vec<ContentBlock>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AssistantMessage {
    pub role: AssistantRoleTag,
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_stop_reason: Option<String>, // provider's raw finish reason, untouched
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<ErrorKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>, // report-and-continue notices
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    pub model: String,
    pub provider: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FunctionResultMessage {
    pub role: FunctionResultRoleTag,
    pub function_call_id: String,
    pub function_id: String,
    pub content: Vec<ContentBlock>,
    pub details: serde_json::Value,
    pub is_error: bool,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CustomMessage {
    pub role: CustomRoleTag,
    pub custom_type: String, // app-defined discriminator
    pub content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub timestamp: i64,
}

/// The canonical transcript message union. Untagged: the single-variant role
/// tags disambiguate deserialization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum AgentMessage {
    Assistant(AssistantMessage),
    FunctionResult(FunctionResultMessage),
    Custom(CustomMessage),
    User(UserMessage),
}
