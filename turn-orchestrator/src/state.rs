//! Persisted turn state. Loaded and saved on every `turn::step_requested`
//! transition. See `docs/plans/2026-04-30-durable-harness-p2.md` § "TurnStateRecord".

use harness_types::{AgentMessage, AssistantMessage, ToolCall, ToolResultMessage};
use serde::{Deserialize, Serialize};

/// Each state corresponds to a node in the durable state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnState {
    Provisioning,
    AwaitingAssistant,
    AssistantStreaming,
    AssistantFinished,
    ToolPrepare,
    ToolExecute,
    ToolFinalize,
    SteeringCheck,
    TearingDown,
    Stopped,
}

impl TurnState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::AwaitingAssistant => "awaiting_assistant",
            Self::AssistantStreaming => "assistant_streaming",
            Self::AssistantFinished => "assistant_finished",
            Self::ToolPrepare => "tool_prepare",
            Self::ToolExecute => "tool_execute",
            Self::ToolFinalize => "tool_finalize",
            Self::SteeringCheck => "steering_check",
            Self::TearingDown => "tearing_down",
            Self::Stopped => "stopped",
        }
    }
}

/// Per-session persisted record. The full transcript lives in
/// `session/<id>/messages` separately to keep this record small.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnStateRecord {
    pub session_id: String,
    pub state: TurnState,
    pub turn_count: u32,
    pub max_turns: Option<u32>,
    pub last_assistant: Option<AssistantMessage>,
    pub pending_tool_calls: Vec<ToolCall>,
    pub tool_results: Vec<ToolResultMessage>,
    /// Set true at any point a `TurnEnd` is emitted; reset false at the
    /// next `TurnStart`. Coordinates emission across handlers so the
    /// stream mirrors legacy `run_loop` (one TurnEnd per turn). See
    /// `harness-runtime/src/loop_state.rs:194-200`.
    #[serde(default)]
    pub turn_end_emitted: bool,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
}

impl TurnStateRecord {
    pub fn new(session_id: impl Into<String>, max_turns: Option<u32>) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            session_id: session_id.into(),
            state: TurnState::Provisioning,
            turn_count: 0,
            max_turns,
            last_assistant: None,
            pending_tool_calls: Vec::new(),
            tool_results: Vec::new(),
            turn_end_emitted: false,
            started_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn transition_to(&mut self, next: TurnState) {
        self.state = next;
        self.updated_at_ms = chrono::Utc::now().timestamp_millis();
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.state, TurnState::Stopped)
    }
}

/// Where the loop's full transcript is persisted.
pub fn messages_key(session_id: &str) -> String {
    format!("session/{session_id}/messages")
}

pub fn turn_state_key(session_id: &str) -> String {
    format!("session/{session_id}/turn_state")
}

pub fn run_request_key(session_id: &str) -> String {
    format!("session/{session_id}/run_request")
}

pub fn cwd_key(session_id: &str) -> String {
    format!("session/{session_id}/cwd")
}

pub fn cwd_index_key(cwd_hash: &str) -> String {
    format!("harness/cwd/{cwd_hash}/last_session_id")
}

pub fn sandbox_id_key(session_id: &str) -> String {
    format!("session/{session_id}/sandbox_id")
}

pub fn tool_schemas_key(session_id: &str) -> String {
    format!("session/{session_id}/tool_schemas")
}

#[allow(dead_code)]
fn _ensure_message_types_in_scope(_: AgentMessage) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_record_starts_in_provisioning() {
        let r = TurnStateRecord::new("s1", Some(32));
        assert_eq!(r.state, TurnState::Provisioning);
        assert_eq!(r.session_id, "s1");
        assert_eq!(r.max_turns, Some(32));
        assert!(!r.is_terminal());
    }

    #[test]
    fn transition_to_stopped_marks_terminal() {
        let mut r = TurnStateRecord::new("s1", None);
        r.transition_to(TurnState::Stopped);
        assert!(r.is_terminal());
    }

    #[test]
    fn turn_state_serde_uses_snake_case() {
        let s = serde_json::to_value(TurnState::AwaitingAssistant).unwrap();
        assert_eq!(s, serde_json::json!("awaiting_assistant"));
    }

    #[test]
    fn keys_use_session_namespace() {
        assert_eq!(turn_state_key("abc"), "session/abc/turn_state");
        assert_eq!(messages_key("abc"), "session/abc/messages");
        assert_eq!(sandbox_id_key("abc"), "session/abc/sandbox_id");
        assert_eq!(tool_schemas_key("abc"), "session/abc/tool_schemas");
        assert_eq!(run_request_key("abc"), "session/abc/run_request");
        assert_eq!(cwd_key("abc"), "session/abc/cwd");
    }

    #[test]
    fn cwd_index_key_uses_sha_namespace() {
        assert_eq!(
            cwd_index_key("abc123"),
            "harness/cwd/abc123/last_session_id"
        );
    }
}
