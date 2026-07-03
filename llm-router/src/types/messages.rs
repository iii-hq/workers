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

/// Reorder function results displaced behind interleaved user messages.
///
/// A notification or steering message injected while a call window is open
/// (e.g. a parked `harness::spawn`) lands between an assistant's
/// `function_call` and its `function_result` in the durable transcript. Every
/// provider wire format requires results directly after the emitting
/// assistant message (Anthropic 400: "tool_use ids were found without
/// tool_result blocks immediately after"), so each provider's wire mapper
/// runs this pass first: move every result up to directly follow its call's
/// assistant message, preserving relative result order and leaving everything
/// else in place. Results whose call is absent (compaction cut it) keep their
/// original position.
pub fn reorder_displaced_results(messages: &[AgentMessage]) -> Vec<&AgentMessage> {
    use std::collections::{HashMap, HashSet};
    let mut call_owner: HashMap<&str, usize> = HashMap::new();
    for (i, m) in messages.iter().enumerate() {
        if let AgentMessage::Assistant(a) = m {
            for b in &a.content {
                if let ContentBlock::FunctionCall { id, .. } = b {
                    call_owner.insert(id.as_str(), i);
                }
            }
        }
    }
    let mut attached: HashMap<usize, Vec<&AgentMessage>> = HashMap::new();
    let mut moved: HashSet<usize> = HashSet::new();
    for (i, m) in messages.iter().enumerate() {
        if let AgentMessage::FunctionResult(r) = m {
            if let Some(&owner) = call_owner.get(r.function_call_id.as_str()) {
                if owner < i {
                    attached.entry(owner).or_default().push(m);
                    moved.insert(i);
                }
            }
        }
    }
    if moved.is_empty() {
        return messages.iter().collect();
    }
    let mut out = Vec::with_capacity(messages.len());
    for (i, m) in messages.iter().enumerate() {
        if !moved.contains(&i) {
            out.push(m);
        }
        if let Some(results) = attached.remove(&i) {
            out.extend(results);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::events::StopReason;

    fn assistant(content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content,
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: "p".into(),
            timestamp: 1,
        })
    }
    fn user_text(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            timestamp: 2,
        })
    }
    fn result(id: &str) -> AgentMessage {
        AgentMessage::FunctionResult(FunctionResultMessage {
            role: FunctionResultRoleTag::FunctionResult,
            function_call_id: id.into(),
            function_id: "f".into(),
            content: vec![],
            details: serde_json::Value::Null,
            is_error: false,
            timestamp: 3,
        })
    }
    fn call(id: &str) -> ContentBlock {
        ContentBlock::FunctionCall {
            id: id.into(),
            function_id: "f".into(),
            arguments: serde_json::json!({}),
        }
    }
    fn is_result(m: &AgentMessage, id: &str) -> bool {
        matches!(m, AgentMessage::FunctionResult(r) if r.function_call_id == id)
    }

    #[test]
    fn displaced_result_moves_directly_after_its_assistant() {
        let msgs = vec![
            assistant(vec![call("t1")]),
            user_text("[notification] progress"),
            result("t1"),
        ];
        let out = reorder_displaced_results(&msgs);
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0], AgentMessage::Assistant(_)));
        assert!(is_result(out[1], "t1"));
        assert!(matches!(out[2], AgentMessage::User(_)));
    }

    #[test]
    fn adjacent_results_and_unmatched_results_keep_positions() {
        let msgs = vec![
            assistant(vec![call("t1"), call("t2")]),
            result("t1"),
            result("t2"),
            result("orphan"),
            user_text("hi"),
        ];
        let out = reorder_displaced_results(&msgs);
        assert!(is_result(out[1], "t1"));
        assert!(is_result(out[2], "t2"));
        assert!(is_result(out[3], "orphan"));
        assert!(matches!(out[4], AgentMessage::User(_)));
    }
}
