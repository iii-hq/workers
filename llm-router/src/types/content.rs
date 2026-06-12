use serde::{Deserialize, Serialize};

/// Content blocks — the atomic units of message content (README § Content blocks).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        mime: String,
        data: String,
    }, // base64
    Thinking {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    FunctionCall {
        id: String,          // unique per call, echoed by the result
        function_id: String, // the iii function id to invoke
        arguments: serde_json::Value,
    },
    FunctionResult {
        function_call_id: String,
        content: Vec<ContentBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}
