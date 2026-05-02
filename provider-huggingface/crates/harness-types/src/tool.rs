use serde::{Deserialize, Serialize};

use crate::content::ContentBlock;

/// A tool definition advertised to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTool {
    pub name: String,
    pub description: String,
    /// JSON schema for the parameters object.
    pub parameters: serde_json::Value,
    pub label: String,
    #[serde(default)]
    pub execution_mode: ExecutionMode,
    #[serde(default)]
    pub prepare_arguments_supported: bool,
}

/// How tool calls in a single assistant message are scheduled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Tool calls run concurrently. Default.
    #[default]
    Parallel,
    /// Tool calls run one at a time. Any tool flagged sequential forces the
    /// whole batch to run sequentially.
    Sequential,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Sse,
    Websocket,
    #[default]
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheRetention {
    None,
    #[default]
    Short,
    Long,
}

/// A single tool-call request emitted by an assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Result of executing a tool. `terminate` is a hint that the loop should end
/// after the current batch; honored only when EVERY tool in the batch sets it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<ContentBlock>,
    pub details: serde_json::Value,
    #[serde(default)]
    pub terminate: bool,
}

/// Outcome of `prepare_tool`. Either ready to execute, or short-circuited by
/// validation failure or a `before_tool_call` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PreparedToolCall {
    Prepared {
        tool_call: ToolCall,
        tool: AgentTool,
        args: serde_json::Value,
    },
    Immediate {
        result: ToolResult,
        is_error: bool,
    },
}

/// Tool call after `after_tool_call` subscribers have run and merged results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalizedToolCall {
    pub tool_call: ToolCall,
    pub result: ToolResult,
    pub is_error: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_mode_default_is_parallel() {
        assert_eq!(ExecutionMode::default(), ExecutionMode::Parallel);
    }

    #[test]
    fn tool_call_roundtrip() {
        let call = ToolCall {
            id: "x".into(),
            name: "read".into(),
            arguments: serde_json::json!({}),
        };
        let json = serde_json::to_string(&call).unwrap();
        let back: ToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(call, back);
    }
}
