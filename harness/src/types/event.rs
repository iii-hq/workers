//! Streaming vocabulary and shared scalars (README § Streaming events).
//! Wire-identical to `llm-router` so relayed frames parse without
//! translation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::message::AssistantMessage;

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
    pub cost_usd: Option<f64>,
}

/// The frozen streaming vocabulary relayed by `llm-router`. The harness
/// reads these frames off the `router::chat` channel to build the
/// assistant message.
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
        /// Legacy fat-frame snapshot; slim producers omit it and readers
        /// accumulate `delta`s from the last block-boundary snapshot.
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
    Ping,
    Stop {
        stop_reason: StopReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
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
