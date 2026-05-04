use serde::{Deserialize, Serialize};

use crate::agent_message::AssistantMessage;

/// Why the model stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    End,
    Length,
    Tool,
    Aborted,
    Error,
}

/// Classification of a streaming error. Enables fallback policy decisions in
/// router workers and re-login flows in clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    AuthExpired,
    RateLimited,
    ContextOverflow,
    Transient,
    Permanent,
}

/// Token accounting for a single streamed response.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_write: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// One streaming event emitted by a provider during an assistant response.
///
/// Provider workers MUST emit these into the iii stream. The loop assembles
/// the final `AssistantMessage` from the sequence. `done` and `error` are
/// terminal. Streams MUST NOT throw; failures are encoded as the final
/// `error` variant with `error_kind` populated.
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
    ToolcallStart {
        partial: AssistantMessage,
    },
    ToolcallDelta {
        partial: AssistantMessage,
        delta: String,
    },
    ToolcallEnd {
        partial: AssistantMessage,
    },
    Usage(Usage),
    Stop {
        stop_reason: StopReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_kind: Option<ErrorKind>,
    },
    Done {
        message: AssistantMessage,
    },
    Error {
        error: AssistantMessage,
    },
}

impl AssistantMessageEvent {
    /// True when this is the last event the provider emits.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Error { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_default_is_zero() {
        let u = Usage::default();
        assert_eq!(u.input, 0);
        assert_eq!(u.output, 0);
    }

    #[test]
    fn done_is_terminal() {
        let ev = AssistantMessageEvent::Stop {
            stop_reason: StopReason::End,
            error_message: None,
            error_kind: None,
        };
        assert!(!ev.is_terminal());
    }

    #[test]
    fn error_kind_serialises_snake() {
        let json = serde_json::to_string(&ErrorKind::ContextOverflow).unwrap();
        assert_eq!(json, "\"context_overflow\"");
    }
}
