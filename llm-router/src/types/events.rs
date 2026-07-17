use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::messages::AssistantMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    End,
    Length,
    FunctionCall,
    Aborted,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
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
///
/// Block-boundary frames (Start / *Start / *End) carry a required
/// cumulative `partial` snapshot — thinking signatures and finalized
/// function-call arguments exist only there. The three per-chunk delta
/// variants carry only `delta`: a cumulative snapshot per chunk made every
/// stream O(N²) in output length (providers rebuilt + re-serialized the
/// whole message per delta; every hop re-parsed it). Readers reconstruct
/// the cumulative message as last-boundary-snapshot + accumulated deltas
/// (see `chat::accumulate`); a legacy fat delta (`partial: Some`) is
/// honored as an authoritative snapshot for old producers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantMessageEvent {
    Start {
        partial: AssistantMessage,
    },
    TextStart {
        partial: AssistantMessage,
    },
    TextDelta {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        partial: Option<AssistantMessage>,
        delta: String,
    },
    TextEnd {
        partial: AssistantMessage,
    },
    ThinkingStart {
        partial: AssistantMessage,
    },
    ThinkingDelta {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        partial: Option<AssistantMessage>,
        delta: String,
    },
    ThinkingEnd {
        partial: AssistantMessage,
    },
    FunctioncallStart {
        partial: AssistantMessage,
    },
    FunctioncallDelta {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        partial: Option<AssistantMessage>,
        delta: String,
        /// Call id receiving this delta; empty from pre-id producers.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        id: String,
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

    /// The ten content-bearing variants: block boundaries and deltas.
    pub fn is_content(&self) -> bool {
        matches!(
            self,
            AssistantMessageEvent::Start { .. }
                | AssistantMessageEvent::TextStart { .. }
                | AssistantMessageEvent::TextDelta { .. }
                | AssistantMessageEvent::TextEnd { .. }
                | AssistantMessageEvent::ThinkingStart { .. }
                | AssistantMessageEvent::ThinkingDelta { .. }
                | AssistantMessageEvent::ThinkingEnd { .. }
                | AssistantMessageEvent::FunctioncallStart { .. }
                | AssistantMessageEvent::FunctioncallDelta { .. }
                | AssistantMessageEvent::FunctioncallEnd { .. }
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
                    partial: Some(partial()),
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
    fn slim_and_fat_delta_frames_both_round_trip() {
        // Slim (post-contract-change producer shape): no partial key at all.
        let slim = serde_json::json!({ "type": "text_delta", "delta": "x" });
        let ev: AssistantMessageEvent = serde_json::from_value(slim.clone()).unwrap();
        assert!(matches!(
            &ev,
            AssistantMessageEvent::TextDelta { partial: None, .. }
        ));
        // partial: None is omitted on the wire, not serialized as null.
        assert_eq!(serde_json::to_value(&ev).unwrap(), slim);

        // Fat (legacy producer shape): partial still parses and re-emits.
        let fat = serde_json::json!({
            "type": "functioncall_delta",
            "partial": serde_json::to_value(partial()).unwrap(),
            "delta": "{\"x\":",
            "id": "c1"
        });
        let ev: AssistantMessageEvent = serde_json::from_value(fat.clone()).unwrap();
        assert!(matches!(
            &ev,
            AssistantMessageEvent::FunctioncallDelta {
                partial: Some(_),
                ..
            }
        ));
        assert_eq!(serde_json::to_value(&ev).unwrap(), fat);
    }

    #[test]
    fn is_content_covers_exactly_the_ten_content_variants() {
        assert!(AssistantMessageEvent::Start { partial: partial() }.is_content());
        assert!(AssistantMessageEvent::TextDelta {
            partial: None,
            delta: "x".into()
        }
        .is_content());
        assert!(AssistantMessageEvent::FunctioncallEnd { partial: partial() }.is_content());
        assert!(!AssistantMessageEvent::Ping.is_content());
        assert!(!AssistantMessageEvent::Usage {
            usage: Usage::default()
        }
        .is_content());
        assert!(!AssistantMessageEvent::Done { message: partial() }.is_content());
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
