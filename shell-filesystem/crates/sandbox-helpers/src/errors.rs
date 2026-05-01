//! Map `sandbox::*` error responses into `ToolResult`-friendly text.

use harness_types::{ContentBlock, TextContent, ToolResult};
use iii_sdk::IIIError;
use serde_json::json;

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
}
