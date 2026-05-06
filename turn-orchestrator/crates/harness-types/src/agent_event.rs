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
    /// A tool call is paused by an approval subscriber, awaiting user decision.
    ApprovalRequested {
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
        /// Unix milliseconds. After this point the gate auto-denies.
        expires_at: u64,
    },
    /// Approval gate has resolved a previously-requested approval.
    ApprovalResolved {
        tool_call_id: String,
        /// "allow" or "deny".
        decision: String,
        /// Free-form reason — populated for "deny" (e.g. "timeout", "user").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
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

    #[test]
    fn approval_requested_round_trips() {
        let evt = AgentEvent::ApprovalRequested {
            tool_call_id: "tc-9".into(),
            tool_name: "shell::filesystem::write".into(),
            args: serde_json::json!({ "path": "/tmp/x" }),
            expires_at: 1_700_000_000_000,
        };
        let json = serde_json::to_value(&evt).unwrap();
        assert_eq!(json["type"], "approval_requested");
        assert_eq!(json["tool_call_id"], "tc-9");
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, evt);
    }

    #[test]
    fn approval_resolved_round_trips_with_optional_reason() {
        let evt = AgentEvent::ApprovalResolved {
            tool_call_id: "tc-9".into(),
            decision: "deny".into(),
            reason: Some("timeout".into()),
        };
        let json = serde_json::to_value(&evt).unwrap();
        assert_eq!(json["type"], "approval_resolved");
        assert_eq!(json["decision"], "deny");
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, evt);

        let none_reason = AgentEvent::ApprovalResolved {
            tool_call_id: "tc-9".into(),
            decision: "allow".into(),
            reason: None,
        };
        let json = serde_json::to_value(&none_reason).unwrap();
        assert!(json.get("reason").map_or(true, |v| v.is_null()));
    }
}
