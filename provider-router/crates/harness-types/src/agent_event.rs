use serde::{Deserialize, Serialize};

use crate::agent_message::{AgentMessage, ToolResultMessage};
use crate::stream_event::AssistantMessageEvent;
use crate::tool::ToolResult;

/// Stable wire format emitted by the loop on `agent::events/<session_id>`.
/// UIs and observers consume this verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Loop has begun processing for this session.
    AgentStart,
    /// Loop has completed; carries the full message tail produced.
    AgentEnd { messages: Vec<AgentMessage> },

    /// One assistant turn (LLM response + any tool calls/results) has begun.
    TurnStart,
    /// One assistant turn has completed.
    TurnEnd {
        message: AgentMessage,
        tool_results: Vec<ToolResultMessage>,
    },

    /// A user, assistant, or tool-result message is about to be added to the transcript.
    MessageStart { message: AgentMessage },
    /// Streaming update on the in-flight assistant message. Only emitted while the
    /// LLM is producing the current response.
    MessageUpdate {
        message: AgentMessage,
        llm_event: AssistantMessageEvent,
    },
    /// The message is final and committed to the transcript.
    MessageEnd { message: AgentMessage },

    /// A tool call has been validated and dispatch has begun.
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
    },
    /// Streaming partial result from a long-running tool.
    ToolExecutionUpdate {
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
        partial_result: serde_json::Value,
    },
    /// Tool execution has finished. `result` is post-`after_tool_call` merged.
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        result: ToolResult,
        is_error: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_start_serialises_with_tag() {
        let json = serde_json::to_string(&AgentEvent::AgentStart).unwrap();
        assert_eq!(json, r#"{"type":"agent_start"}"#);
    }

    #[test]
    fn tool_start_carries_args() {
        let ev = AgentEvent::ToolExecutionStart {
            tool_call_id: "id".into(),
            tool_name: "read".into(),
            args: serde_json::json!({ "path": "/x" }),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }
}
