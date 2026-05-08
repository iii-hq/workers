use serde::{Deserialize, Serialize};

use crate::content::ContentBlock;

/// A function slot advertised to the model (harness uses `agent_call`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentFunction {
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

/// How function calls in a single assistant message are scheduled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Calls run concurrently. Default.
    #[default]
    Parallel,
    /// Calls run one at a time. Any call flagged sequential forces the
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

/// A single function-call request emitted by an assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub id: String,
    /// iii function id (e.g. `shell::filesystem::ls`).
    #[serde(alias = "name")]
    pub function_id: String,
    pub arguments: serde_json::Value,
}

/// Result of executing a function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionResult {
    pub content: Vec<ContentBlock>,
    pub details: serde_json::Value,
    #[serde(default)]
    pub terminate: bool,
}

/// Outcome of prepare. Either ready to execute, or short-circuited.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PreparedFunctionCall {
    Prepared {
        #[serde(alias = "tool_call")]
        function_call: FunctionCall,
        #[serde(alias = "tool")]
        function: AgentFunction,
        args: serde_json::Value,
    },
    Immediate {
        result: FunctionResult,
        is_error: bool,
    },
}

/// After `after_function_call` subscribers have merged results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalizedFunctionCall {
    #[serde(alias = "tool_call")]
    pub function_call: FunctionCall,
    pub result: FunctionResult,
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
    fn function_call_roundtrip() {
        let call = FunctionCall {
            id: "x".into(),
            function_id: "read".into(),
            arguments: serde_json::json!({}),
        };
        let json = serde_json::to_string(&call).unwrap();
        let back: FunctionCall = serde_json::from_str(&json).unwrap();
        assert_eq!(call, back);
    }

    #[test]
    fn function_call_deserializes_legacy_name_field() {
        let json = r#"{"id":"x","name":"shell::filesystem::ls","arguments":{}}"#;
        let call: FunctionCall = serde_json::from_str(json).unwrap();
        assert_eq!(call.function_id, "shell::filesystem::ls");
    }
}
