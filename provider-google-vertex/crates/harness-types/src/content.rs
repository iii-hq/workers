use serde::{Deserialize, Serialize};

use crate::thinking::TextSignature;

/// A block of content. Carried by user, assistant, and tool-result messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContentBlock {
    Text(TextContent),
    Image(ImageContent),
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    ToolResult {
        tool_call_id: String,
        content: Vec<ContentBlock>,
        is_error: bool,
    },
    Thinking {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<TextSignature>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextContent {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageContent {
    /// MIME type, for example `image/png` or `image/jpeg`.
    pub mime: String,
    /// Base64-encoded image bytes.
    pub data: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_block_roundtrip() {
        let block = ContentBlock::Text(TextContent {
            text: "hello".into(),
        });
        let json = serde_json::to_string(&block).unwrap();
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, back);
    }

    #[test]
    fn tool_call_block_roundtrip() {
        let block = ContentBlock::ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: serde_json::json!({ "path": "/tmp/x" }),
        };
        let json = serde_json::to_string(&block).unwrap();
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, back);
    }
}
