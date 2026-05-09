use serde::{Deserialize, Serialize};

use crate::agent_message::{AgentMessage, FunctionResultMessage};
use crate::function::FunctionResult;
use crate::stream_event::AssistantMessageEvent;

/// Outcome of an approval gate. Wire format is the lowercase string
/// `"allow"` or `"deny"`; the typed enum prevents constructing illegal values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalDecision {
    Allow,
    Deny,
}

/// Stable wire format emitted by the loop on `agent::events/<session_id>`.
/// UIs and observers consume this verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Loop has begun processing for this session.
    AgentStart,
    /// Loop has completed; carries the full message tail produced.
    AgentEnd { messages: Vec<AgentMessage> },

    /// One assistant turn (LLM response + any function calls/results) has begun.
    TurnStart,
    /// One assistant turn has completed.
    TurnEnd {
        message: AgentMessage,
        #[serde(alias = "tool_results")]
        function_results: Vec<FunctionResultMessage>,
    },

    /// A user, assistant, or function-result message is about to be added to the transcript.
    MessageStart { message: AgentMessage },
    /// Streaming update on the in-flight assistant message. Only emitted while the
    /// LLM is producing the current response.
    MessageUpdate {
        message: AgentMessage,
        llm_event: AssistantMessageEvent,
    },
    /// The message is final and committed to the transcript.
    MessageEnd { message: AgentMessage },

    /// A function call has been validated and dispatch has begun.
    #[serde(rename = "function_execution_start", alias = "tool_execution_start")]
    FunctionExecutionStart {
        #[serde(alias = "tool_call_id")]
        function_call_id: String,
        #[serde(alias = "tool_name")]
        function_id: String,
        args: serde_json::Value,
    },
    /// Streaming partial result from a long-running function.
    #[serde(rename = "function_execution_update", alias = "tool_execution_update")]
    FunctionExecutionUpdate {
        #[serde(alias = "tool_call_id")]
        function_call_id: String,
        #[serde(alias = "tool_name")]
        function_id: String,
        args: serde_json::Value,
        partial_result: serde_json::Value,
    },
    /// Function execution has finished. `result` is post-`after_function_call` merged.
    #[serde(rename = "function_execution_end", alias = "tool_execution_end")]
    FunctionExecutionEnd {
        #[serde(alias = "tool_call_id")]
        function_call_id: String,
        #[serde(alias = "tool_name")]
        function_id: String,
        result: FunctionResult,
        is_error: bool,
    },
    /// A function call is paused by an approval subscriber, awaiting user decision.
    ApprovalRequested {
        #[serde(alias = "tool_call_id")]
        function_call_id: String,
        #[serde(alias = "tool_name")]
        function_id: String,
        args: serde_json::Value,
        /// Unix milliseconds. After this point the gate auto-denies.
        expires_at: u64,
    },
    /// Approval gate has resolved a previously-requested approval.
    ApprovalResolved {
        #[serde(alias = "tool_call_id")]
        function_call_id: String,
        decision: ApprovalDecision,
        /// Free-form reason — populated for "deny" (e.g. "timeout", "user").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_message::UserMessage;

    #[test]
    fn agent_start_serialises_with_tag() {
        let json = serde_json::to_string(&AgentEvent::AgentStart).unwrap();
        assert_eq!(json, r#"{"type":"agent_start"}"#);
    }

    #[test]
    fn function_start_carries_args() {
        let ev = AgentEvent::FunctionExecutionStart {
            function_call_id: "id".into(),
            function_id: "read".into(),
            args: serde_json::json!({ "path": "/x" }),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("function_execution_start"));
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn function_execution_start_legacy_type_deserializes() {
        let json = r#"{"type":"tool_execution_start","tool_call_id":"id","tool_name":"read","args":{"path":"/x"}}"#;
        let back: AgentEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            back,
            AgentEvent::FunctionExecutionStart {
                function_call_id: "id".into(),
                function_id: "read".into(),
                args: serde_json::json!({ "path": "/x" }),
            }
        );
    }

    #[test]
    fn approval_requested_round_trips() {
        let evt = AgentEvent::ApprovalRequested {
            function_call_id: "tc-9".into(),
            function_id: "shell::filesystem::write".into(),
            args: serde_json::json!({ "path": "/tmp/x" }),
            expires_at: 1_700_000_000_000,
        };
        let json = serde_json::to_value(&evt).unwrap();
        assert_eq!(json["type"], "approval_requested");
        assert_eq!(json["function_call_id"], "tc-9");
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, evt);
    }

    #[test]
    fn approval_resolved_round_trips_with_optional_reason() {
        let evt = AgentEvent::ApprovalResolved {
            function_call_id: "tc-9".into(),
            decision: ApprovalDecision::Deny,
            reason: Some("timeout".into()),
        };
        let json = serde_json::to_value(&evt).unwrap();
        assert_eq!(json["type"], "approval_resolved");
        assert_eq!(json["decision"], "deny");
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, evt);

        let none_reason = AgentEvent::ApprovalResolved {
            function_call_id: "tc-9".into(),
            decision: ApprovalDecision::Allow,
            reason: None,
        };
        let json = serde_json::to_value(&none_reason).unwrap();
        assert_eq!(json["decision"], "allow");
        assert!(
            !json.as_object().unwrap().contains_key("reason"),
            "reason should be omitted when None: {json}"
        );
    }

    #[test]
    fn turn_end_legacy_tool_results_field() {
        let msg = AgentMessage::User(UserMessage {
            content: vec![],
            timestamp: 0,
        });
        let json = serde_json::json!({
            "type": "turn_end",
            "message": msg,
            "tool_results": []
        });
        let evt: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(evt, AgentEvent::TurnEnd { .. }));
    }
}
