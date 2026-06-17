//! Content blocks — the atomic units of message content
//! (README § Content blocks). Wire-identical to `llm-router` and
//! `session-manager` so messages round-trip across the bus untouched.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        mime: String,
        data: String,
    },
    Thinking {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// Opaque redacted thinking payload — replayed verbatim on the wire.
    RedactedThinking {
        data: String,
    },
    FunctionCall {
        id: String,
        function_id: String,
        arguments: serde_json::Value,
    },
    FunctionResult {
        function_call_id: String,
        content: Vec<ContentBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

impl ContentBlock {
    /// A single text block (the common case for user/system content).
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }

    /// Concatenated text of all `Text` blocks in a slice, newline-joined.
    pub fn join_text(blocks: &[ContentBlock]) -> String {
        let mut out = String::new();
        for b in blocks {
            if let ContentBlock::Text { text } = b {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
        out
    }
}
