use serde::{Deserialize, Serialize};

use crate::types::messages::AssistantMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    End,
    Length,
    FunctionCall,
    Aborted,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    AuthExpired,
    RateLimited,
    ContextOverflow,
    Transient,
    Permanent,
}

impl ErrorKind {
    /// One retry policy, in the router, nowhere else (spec § Retries).
    pub fn is_retryable(self) -> bool {
        matches!(self, ErrorKind::RateLimited | ErrorKind::Transient)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>, // filled by llm-router from catalog pricing
}

/// The frozen 15-variant streaming vocabulary (README § Streaming events).
/// New frame types are a contract revision, not a provider choice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantMessageEvent {
    Start {
        partial: AssistantMessage,
    },
    TextStart {
        partial: AssistantMessage,
    },
    TextDelta {
        partial: AssistantMessage,
        delta: String,
    },
    TextEnd {
        partial: AssistantMessage,
    },
    ThinkingStart {
        partial: AssistantMessage,
    },
    ThinkingDelta {
        partial: AssistantMessage,
        delta: String,
    },
    ThinkingEnd {
        partial: AssistantMessage,
    },
    FunctioncallStart {
        partial: AssistantMessage,
    },
    FunctioncallDelta {
        partial: AssistantMessage,
        delta: String,
    },
    FunctioncallEnd {
        partial: AssistantMessage,
    },
    Usage {
        usage: Usage,
    },
    Ping, // liveness heartbeat; consumers ignore
    Stop {
        stop_reason: StopReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_kind: Option<ErrorKind>,
    },
    Done {
        message: AssistantMessage,
    }, // terminal
    Error {
        error: AssistantMessage,
    }, // terminal
}

impl AssistantMessageEvent {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            AssistantMessageEvent::Done { .. } | AssistantMessageEvent::Error { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::messages::{AssistantMessage, AssistantRoleTag};

    fn partial() -> AssistantMessage {
        AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content: vec![],
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: "p".into(),
            timestamp: 1,
        }
    }

    #[test]
    fn event_tags_match_the_spec_wire_strings() {
        let cases: Vec<(AssistantMessageEvent, &str)> = vec![
            (AssistantMessageEvent::Start { partial: partial() }, "start"),
            (
                AssistantMessageEvent::TextDelta {
                    partial: partial(),
                    delta: "x".into(),
                },
                "text_delta",
            ),
            (
                AssistantMessageEvent::FunctioncallStart { partial: partial() },
                "functioncall_start",
            ),
            (
                AssistantMessageEvent::Usage {
                    usage: Usage::default(),
                },
                "usage",
            ),
            (AssistantMessageEvent::Ping, "ping"),
            (AssistantMessageEvent::Done { message: partial() }, "done"),
            (AssistantMessageEvent::Error { error: partial() }, "error"),
        ];
        for (ev, tag) in cases {
            let v = serde_json::to_value(&ev).unwrap();
            assert_eq!(v["type"], tag, "wire tag for {tag}");
        }
    }

    #[test]
    fn spec_example_done_frame_round_trips() {
        let json = serde_json::json!({
            "type": "done",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "Hello" }],
                "stop_reason": "end",
                "model": "claude-sonnet-4",
                "provider": "anthropic",
                "timestamp": 2
            }
        });
        let ev: AssistantMessageEvent = serde_json::from_value(json.clone()).unwrap();
        assert!(ev.is_terminal());
        assert_eq!(serde_json::to_value(&ev).unwrap(), json);
    }

    #[test]
    fn is_terminal_only_for_done_and_error() {
        assert!(AssistantMessageEvent::Done { message: partial() }.is_terminal());
        assert!(AssistantMessageEvent::Error { error: partial() }.is_terminal());
        assert!(!AssistantMessageEvent::Ping.is_terminal());
        assert!(!AssistantMessageEvent::Usage {
            usage: Usage::default()
        }
        .is_terminal());
    }
}
