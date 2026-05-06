//! Map `sandbox::*` error responses into `ToolResult`-friendly text.
//!
//! `ContentBlock`, `TextContent`, and `ToolResult` are inlined locally
//! (rather than vendoring `harness-types`) since the shell workers only
//! ever construct the `Text` variant. Wire format matches the canonical
//! `harness_types` shapes byte-for-byte — downstream consumers
//! (turn-orchestrator, agents) deserialize these results back as
//! `harness_types::ToolResult` without any conversion.

use iii_sdk::IIIError;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Wire-compatible mirror of `harness_types::ContentBlock`. Only the
/// variants the shell workers actually produce are inlined here.
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

/// Wire-compatible mirror of `harness_types::ToolResult`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<ContentBlock>,
    pub details: serde_json::Value,
    #[serde(default)]
    pub terminate: bool,
}

#[derive(Debug)]
pub struct ShellError {
    pub code: String,
    pub message: String,
}

impl ShellError {
    pub fn missing_sandbox() -> Self {
        Self {
            code: "MissingSandbox".into(),
            message: "no sandbox_id in args and none in session state — call sandbox::create first"
                .into(),
        }
    }

    /// Wrap any iii bus failure as `S000` — a catch-all for transient or
    /// structural errors crossing the shell↔sandbox boundary. Specific
    /// `S1xx`/`S2xx`/`S3xx`/`S4xx` codes from the sandbox daemon (see
    /// `iii/crates/iii-worker/src/sandbox_daemon/README.md`) ride through
    /// in the underlying `IIIError`'s `Display`. P4+ may unpack these into
    /// distinct codes once the iii SDK exposes structured error variants.
    pub fn from_iii(err: IIIError) -> Self {
        Self {
            code: "S000".into(),
            message: err.to_string(),
        }
    }

    pub fn to_tool_result(&self) -> ToolResult {
        ToolResult {
            content: vec![ContentBlock::Text(TextContent {
                text: format!("{}: {}", self.code, self.message),
            })],
            details: json!({ "error": { "code": self.code, "message": self.message } }),
            terminate: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_sandbox_renders_a_user_visible_message() {
        let err = ShellError::missing_sandbox();
        let result = err.to_tool_result();
        let text = match result.content.first().unwrap() {
            ContentBlock::Text(t) => &t.text,
            _ => panic!("expected text content"),
        };
        assert!(text.contains("MissingSandbox"));
        assert!(text.contains("sandbox::create"));
    }

    #[test]
    fn text_block_serializes_with_canonical_tag() {
        let block = ContentBlock::Text(TextContent {
            text: "hello".into(),
        });
        let v = serde_json::to_value(&block).unwrap();
        assert_eq!(v["type"], "text");
        assert_eq!(v["text"], "hello");
    }

    #[test]
    fn tool_result_round_trips_through_json() {
        let r = ToolResult {
            content: vec![ContentBlock::Text(TextContent { text: "x".into() })],
            details: json!({}),
            terminate: false,
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: ToolResult = serde_json::from_value(v).unwrap();
        assert_eq!(r, back);
    }
}
