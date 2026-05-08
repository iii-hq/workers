use serde::{Deserialize, Serialize};

use crate::thinking::TextSignature;

/// A block of content. Carried by user, assistant, and function-result messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContentBlock {
    Text(TextContent),
    Image(ImageContent),
    #[serde(rename = "functionCall", alias = "toolCall")]
    FunctionCall {
        id: String,
        #[serde(alias = "name")]
        function_id: String,
        arguments: serde_json::Value,
    },
    #[serde(rename = "functionResult", alias = "toolResult")]
    FunctionResult {
        #[serde(alias = "tool_call_id")]
        function_call_id: String,
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
    fn function_call_block_roundtrip() {
        let block = ContentBlock::FunctionCall {
            id: "call_1".into(),
            function_id: "read".into(),
            arguments: serde_json::json!({ "path": "/tmp/x" }),
        };
        let json = serde_json::to_string(&block).unwrap();
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, back);
    }

    #[test]
    fn function_call_block_legacy_tool_call_type() {
        let json = r#"{"type":"toolCall","id":"call_1","name":"read","arguments":{"path":"/tmp/x"}}"#;
        let back: ContentBlock = serde_json::from_str(json).unwrap();
        assert_eq!(
            back,
            ContentBlock::FunctionCall {
                id: "call_1".into(),
                function_id: "read".into(),
                arguments: serde_json::json!({ "path": "/tmp/x" }),
            }
        );
    }
}
