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
    /// One retry policy, in the router, nowhere else (spec § Retries). A
    /// `rate_limited` whose [`ErrorDetails::usage_limit`] is set is the
    /// exception: see [`crate::types::messages::AssistantMessage::error_is_retryable`].
    pub fn is_retryable(self) -> bool {
        matches!(self, ErrorKind::RateLimited | ErrorKind::Transient)
    }
}

/// The usage window an account's limit belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsageWindow {
    /// The rolling five-hour window.
    FiveHour,
    /// The weekly window (any of the plan's seven-day limits).
    Week,
}

/// What an error's kind alone does not say. Optional on the assistant message
/// and on the router's `ErrorShape`, and serialized only when present, so
/// every reader that does not know it ignores it: `ErrorKind` itself keeps its
/// five values, because its copies elsewhere deserialize it strictly.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorDetails {
    /// The account's usage limit is reached: a `rate_limited` that waiting a
    /// moment does not fix, so the router never retries it.
    #[serde(default)]
    pub usage_limit: bool,
    /// Which limit was reached, when the provider said so. An unknown value
    /// reads as absent.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_window"
    )]
    pub window: Option<UsageWindow>,
    /// When that limit resets, in milliseconds since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<u64>,
}

impl ErrorDetails {
    /// An account's usage limit, with what the provider knew of it.
    pub fn usage_limit(window: Option<UsageWindow>, resets_at_ms: Option<u64>) -> Self {
        Self {
            usage_limit: true,
            window,
            resets_at_ms,
        }
    }
}

/// A window this router does not know is absent, not a malformed frame.
fn lenient_window<'de, D>(deserializer: D) -> Result<Option<UsageWindow>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

/// Details this router cannot read are absent: a terminal frame is never
/// dropped as malformed because of them. Each field is read on its own, so a
/// malformed `window` or `resets_at_ms` never costs `usage_limit: true` (a
/// limit read as a busy moment would be retried against it).
pub(crate) fn lenient_error_details<'de, D>(
    deserializer: D,
) -> Result<Option<ErrorDetails>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.as_ref().and_then(read_error_details))
}

/// `error_details` field by field: an object, else nothing.
fn read_error_details(value: &serde_json::Value) -> Option<ErrorDetails> {
    let object = value.as_object()?;
    Some(ErrorDetails {
        usage_limit: object
            .get("usage_limit")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        window: object
            .get("window")
            .and_then(|window| serde_json::from_value(window.clone()).ok()),
        resets_at_ms: object
            .get("resets_at_ms")
            .and_then(serde_json::Value::as_u64),
    })
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

/// Small, bounded view of the top-level agent_call identity observed while
/// its JSON arguments are still forming. The router adds this to relayed
/// deltas; providers leave it absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FunctionCallArgumentsPreview {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
        /// Router-produced view of fields useful to an in-flight UI.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments_preview: Option<FunctionCallArgumentsPreview>,
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
    /// Finish a failed generation with its actual output and accounting.
    /// Upstream diagnostics belong in `error_message`.
    pub fn error(
        mut partial: AssistantMessage,
        kind: ErrorKind,
        message: impl Into<String>,
    ) -> Self {
        partial.stop_reason = StopReason::Error;
        partial.error_kind = Some(kind);
        partial.error_message = Some(message.into());
        Self::Error { error: partial }
    }

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
            error_details: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: "p".into(),
            timestamp: 1,
        }
    }

    #[test]
    fn terminal_error_preserves_output_usage_and_provider_metadata() {
        use crate::types::content::ContentBlock;

        let started = AssistantMessage {
            content: vec![ContentBlock::FunctionCall {
                id: "call-1".into(),
                function_id: "lookup".into(),
                arguments: serde_json::json!({"id": 7}),
            }],
            native_stop_reason: Some("tool_calls".into()),
            warnings: Some(vec!["upstream warning".into()]),
            usage: Some(Usage {
                input: Some(12),
                output: Some(3),
                cache_read: Some(2),
                ..Usage::default()
            }),
            ..partial()
        };
        for original in [partial(), started] {
            let expected = AssistantMessageEvent::Error {
                error: AssistantMessage {
                    stop_reason: StopReason::Error,
                    error_kind: Some(ErrorKind::Transient),
                    error_message: Some("connection reset".into()),
                    ..original.clone()
                },
            };
            assert_eq!(
                AssistantMessageEvent::error(original, ErrorKind::Transient, "connection reset"),
                expected
            );
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
        assert!(
            AssistantMessageEvent::TextDelta {
                partial: None,
                delta: "x".into()
            }
            .is_content()
        );
        assert!(AssistantMessageEvent::FunctioncallEnd { partial: partial() }.is_content());
        assert!(!AssistantMessageEvent::Ping.is_content());
        assert!(
            !AssistantMessageEvent::Usage {
                usage: Usage::default()
            }
            .is_content()
        );
        assert!(!AssistantMessageEvent::Done { message: partial() }.is_content());
    }

    #[test]
    fn error_details_wire_shape() {
        let details =
            ErrorDetails::usage_limit(Some(UsageWindow::FiveHour), Some(1_800_000_000_000));
        assert_eq!(
            serde_json::to_value(&details).unwrap(),
            serde_json::json!({
                "usage_limit": true, "window": "five_hour", "resets_at_ms": 1_800_000_000_000u64
            })
        );
        // Optional facts are omitted, never null.
        assert_eq!(
            serde_json::to_value(ErrorDetails::usage_limit(None, None)).unwrap(),
            serde_json::json!({ "usage_limit": true })
        );
        let week: ErrorDetails =
            serde_json::from_value(serde_json::json!({ "usage_limit": true, "window": "week" }))
                .unwrap();
        assert_eq!(week.window, Some(UsageWindow::Week));
    }

    #[test]
    fn an_unknown_window_reads_as_absent() {
        let details: ErrorDetails = serde_json::from_value(serde_json::json!({
            "usage_limit": true, "window": "month", "resets_at_ms": 5
        }))
        .unwrap();
        assert_eq!(details, ErrorDetails::usage_limit(None, Some(5)));
    }

    #[test]
    fn error_frame_carries_details_and_keeps_them_through_error() {
        let mut limited = partial();
        limited.error_details = Some(ErrorDetails::usage_limit(Some(UsageWindow::Week), None));
        let ev = AssistantMessageEvent::error(limited, ErrorKind::RateLimited, "limit");
        let wire = serde_json::to_value(&ev).unwrap();
        assert_eq!(wire["error"]["error_kind"], "rate_limited");
        assert_eq!(
            wire["error"]["error_details"],
            serde_json::json!({ "usage_limit": true, "window": "week" })
        );
        let back: AssistantMessageEvent = serde_json::from_value(wire).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn unreadable_details_never_drop_the_frame() {
        let wire = serde_json::json!({
            "type": "error",
            "error": {
                "role": "assistant", "content": [], "stop_reason": "error",
                "error_kind": "rate_limited", "error_details": "limit",
                "model": "m", "provider": "p", "timestamp": 1
            }
        });
        let AssistantMessageEvent::Error { error } = serde_json::from_value(wire).unwrap() else {
            panic!("not an error frame")
        };
        assert_eq!(error.error_kind, Some(ErrorKind::RateLimited));
        assert_eq!(error.error_details, None);
    }

    #[test]
    fn a_malformed_field_never_costs_the_usage_limit() {
        let read = |details: serde_json::Value| {
            let wire = serde_json::json!({
                "type": "error",
                "error": {
                    "role": "assistant", "content": [], "stop_reason": "error",
                    "error_kind": "rate_limited", "error_details": details,
                    "model": "m", "provider": "p", "timestamp": 1
                }
            });
            let AssistantMessageEvent::Error { error } = serde_json::from_value(wire).unwrap()
            else {
                panic!("not an error frame")
            };
            error.error_details
        };
        // A bad reset or window: the flag and whatever else reads stay.
        assert_eq!(
            read(serde_json::json!({ "usage_limit": true, "resets_at_ms": "soon", "window": 5 })),
            Some(ErrorDetails::usage_limit(None, None))
        );
        assert_eq!(
            read(serde_json::json!({ "usage_limit": true, "resets_at_ms": -1, "window": "week" })),
            Some(ErrorDetails::usage_limit(Some(UsageWindow::Week), None))
        );
        assert_eq!(
            read(
                serde_json::json!({ "usage_limit": true, "resets_at_ms": 1.5, "window": "five_hour" })
            ),
            Some(ErrorDetails::usage_limit(Some(UsageWindow::FiveHour), None))
        );
        // A bad flag is not a limit; the rest still reads.
        assert_eq!(
            read(serde_json::json!({ "usage_limit": "yes", "resets_at_ms": 5, "window": "week" })),
            Some(ErrorDetails {
                usage_limit: false,
                window: Some(UsageWindow::Week),
                resets_at_ms: Some(5),
            })
        );
        assert_eq!(read(serde_json::json!(null)), None);
        assert_eq!(read(serde_json::json!([true])), None);
    }

    #[test]
    fn is_terminal_only_for_done_and_error() {
        assert!(AssistantMessageEvent::Done { message: partial() }.is_terminal());
        assert!(AssistantMessageEvent::Error { error: partial() }.is_terminal());
        assert!(!AssistantMessageEvent::Ping.is_terminal());
        assert!(
            !AssistantMessageEvent::Usage {
                usage: Usage::default()
            }
            .is_terminal()
        );
    }
}
