//! Local mirrors of the llm-router wire types the scripted router speaks.
//!
//! Interop with the harness is JSON-on-the-wire only (each worker keeps its
//! own copy of these enums), so the integration crate mirrors them instead of
//! taking a cargo dependency on llm-router. Fidelity is enforced by golden
//! tests against `llm-router/tests/golden/schemas/` and the spec's literal
//! frame tables. Mirrors: `llm-router/src/types/{events,messages,content,router}.rs`.
//!
//! Unlike the originals, these deny unknown fields: they only ever
//! deserialize authored fixtures, where an unknown field is an authoring
//! error the loader must reject.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum AssistantRoleTag {
    #[serde(rename = "assistant")]
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        mime: String,
        data: String,
    },
    Thinking {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    RedactedThinking {
        data: String,
    },
    FunctionCall {
        id: String,
        function_id: String,
        arguments: serde_json::Value,
    },
    FunctionResult {
        function_call_id: String,
        content: Vec<ContentBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssistantMessage {
    pub role: AssistantRoleTag,
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<ErrorKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    pub model: String,
    pub provider: String,
    pub timestamp: i64,
}

/// The frozen 15-variant streaming vocabulary
/// (`llm-router/src/types/events.rs:53`).
///
/// Delta variants carry `partial` as an Option, mirroring the router: the
/// current wire format is the slim delta (`partial` omitted, readers
/// accumulate); a legacy fat delta (`partial: Some`) is an authoritative
/// snapshot from old producers. Fixtures may author either form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
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
        matches!(self, Self::Done { .. } | Self::Error { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorShape {
    pub code: String,
    pub message: String,
}

/// Mirror of `ChatResponse` (`llm-router/src/types/router.rs:54`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouterChatResponse {
    pub ok: bool,
    pub provider: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorShape>,
}
