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

/// Structured deny payload carried on the `approval_resolved` event.
/// Mirrors the `Denial` type emitted by approval-gate so downstream
/// consumers (UI, audit, the LLM via stitching) can branch on `kind`
/// instead of parsing a free-form reason string.
///
/// Wire shape (serde tag=kind, content=detail, snake_case):
///   `{ "kind": "policy",         "detail": { "classifier_reason": "...", "classifier_fn": "..." } }`
///   `{ "kind": "user_rejected",  "detail": null }`
///   `{ "kind": "user_corrected", "detail": { "feedback": "..." } }`
///   `{ "kind": "state_error",    "detail": { "phase": "...", "error": "..." } }`
///   `{ "kind": "legacy",         "detail": { "reason": "..." } }`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum Denial {
    Policy {
        classifier_reason: String,
        classifier_fn: String,
    },
    UserRejected,
    UserCorrected {
        feedback: String,
    },
    StateError {
        phase: String,
        error: String,
    },
    Legacy {
        reason: String,
    },
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
        /// Structured deny payload — populated when `decision == Deny`,
        /// absent when `decision == Allow` or when the gate timed out
        /// (timed_out is self-describing via the persisted record status).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        denial: Option<Denial>,
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
    fn approval_resolved_round_trips_with_structured_denial() {
        let evt = AgentEvent::ApprovalResolved {
            function_call_id: "tc-9".into(),
            decision: ApprovalDecision::Deny,
            denial: Some(Denial::UserCorrected {
                feedback: "try git diff first".into(),
            }),
        };
        let json = serde_json::to_value(&evt).unwrap();
        assert_eq!(json["type"], "approval_resolved");
        assert_eq!(json["decision"], "deny");
        assert_eq!(json["denial"]["kind"], "user_corrected");
        assert_eq!(json["denial"]["detail"]["feedback"], "try git diff first");
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, evt);

        let none_denial = AgentEvent::ApprovalResolved {
            function_call_id: "tc-9".into(),
            decision: ApprovalDecision::Allow,
            denial: None,
        };
        let json = serde_json::to_value(&none_denial).unwrap();
        assert_eq!(json["decision"], "allow");
        assert!(
            !json.as_object().unwrap().contains_key("denial"),
            "denial should be omitted when None: {json}"
        );
    }

    #[test]
    fn denial_policy_serializes_with_classifier_detail() {
        let d = Denial::Policy {
            classifier_reason: "command matches denylist".into(),
            classifier_fn: "shell::classify_argv".into(),
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["kind"], "policy");
        assert_eq!(v["detail"]["classifier_reason"], "command matches denylist");
        assert_eq!(v["detail"]["classifier_fn"], "shell::classify_argv");
        let back: Denial = serde_json::from_value(v).unwrap();
        assert_eq!(back, d);
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
