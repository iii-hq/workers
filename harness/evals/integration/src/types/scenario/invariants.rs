use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{
    LifecycleExpectationV1, MessageCountsExpectationV1, SendFlagsExpectationV1,
    TerminalExpectationV1,
};

macro_rules! invariant_registry {
    ($($variant:ident => $id:literal),+ $(,)?) => {
        /// Stable registry for every invariant understood by the compiler and grader.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum InvariantKind {
            $($variant),+
        }

        impl InvariantKind {
            pub const ALL: [Self; [$($id),+].len()] = [
                $(Self::$variant),+
            ];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $id),+
                }
            }

            pub fn from_id(id: &str) -> Option<Self> {
                match id {
                    $($id => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

invariant_registry! {
    SendFlags => "send.flags",
    TranscriptMessageCounts => "transcript.message_counts",
    TranscriptAssistantText => "transcript.assistant_text",
    TranscriptFunctionResult => "transcript.function_result",
    TranscriptCallsClosed => "transcript.calls_closed",
    TranscriptNoDuplicates => "transcript.no_duplicates",
    StatusTerminal => "status.terminal",
    LifecycleCompletedOnce => "lifecycle.completed_once",
    RouterGenerationsConsumed => "router.generations_consumed",
    TargetCalls => "target.calls",
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssistantTextInvariantV1 {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FunctionResultInvariantV1 {
    pub function_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerationsConsumedInvariantV1 {
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetCallsInvariantV1 {
    pub function_id: String,
    pub count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_subset: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyInvariantParameters {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DecodedInvariantV1<'a> {
    SendFlags(&'a Map<String, Value>),
    TranscriptMessageCounts(&'a Map<String, Value>),
    TranscriptAssistantText(&'a Map<String, Value>),
    TranscriptFunctionResult(&'a Map<String, Value>),
    TranscriptCallsClosed(&'a Map<String, Value>),
    TranscriptNoDuplicates(&'a Map<String, Value>),
    StatusTerminal(&'a Map<String, Value>),
    LifecycleCompletedOnce(&'a Map<String, Value>),
    RouterGenerationsConsumed(&'a Map<String, Value>),
    TargetCalls(&'a Map<String, Value>),
}

#[derive(Debug, thiserror::Error)]
pub enum InvariantDecodeError {
    #[error("unknown invariant id {0:?}")]
    UnknownId(String),
}

/// Stable wire form persisted in compiled scenarios.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvariantSpecV1 {
    pub id: String,
    #[serde(default)]
    pub parameters: Map<String, Value>,
}

impl InvariantSpecV1 {
    pub fn send_flags(parameters: SendFlagsExpectationV1) -> Self {
        Self::typed(InvariantKind::SendFlags, parameters)
    }

    pub fn transcript_message_counts(parameters: MessageCountsExpectationV1) -> Self {
        Self::typed(InvariantKind::TranscriptMessageCounts, parameters)
    }

    pub fn transcript_assistant_text(text: String) -> Self {
        Self::typed(
            InvariantKind::TranscriptAssistantText,
            AssistantTextInvariantV1 { text, usage: None },
        )
    }

    pub fn transcript_function_result(parameters: FunctionResultInvariantV1) -> Self {
        Self::typed(InvariantKind::TranscriptFunctionResult, parameters)
    }

    pub fn transcript_calls_closed() -> Self {
        Self::typed(
            InvariantKind::TranscriptCallsClosed,
            EmptyInvariantParameters {},
        )
    }

    pub fn transcript_no_duplicates() -> Self {
        Self::typed(
            InvariantKind::TranscriptNoDuplicates,
            EmptyInvariantParameters {},
        )
    }

    pub fn status_terminal(parameters: TerminalExpectationV1) -> Self {
        Self::typed(InvariantKind::StatusTerminal, parameters)
    }

    pub fn lifecycle_completed_once(parameters: LifecycleExpectationV1) -> Self {
        Self::typed(InvariantKind::LifecycleCompletedOnce, parameters)
    }

    pub fn router_generations_consumed(count: u64) -> Self {
        Self::typed(
            InvariantKind::RouterGenerationsConsumed,
            GenerationsConsumedInvariantV1 { count },
        )
    }

    pub fn target_calls(parameters: TargetCallsInvariantV1) -> Self {
        Self::typed(InvariantKind::TargetCalls, parameters)
    }

    pub fn decode(&self) -> Result<DecodedInvariantV1<'_>, InvariantDecodeError> {
        let kind = InvariantKind::from_id(&self.id)
            .ok_or_else(|| InvariantDecodeError::UnknownId(self.id.clone()))?;
        let decoded = match kind {
            InvariantKind::SendFlags => DecodedInvariantV1::SendFlags(&self.parameters),
            InvariantKind::TranscriptMessageCounts => {
                DecodedInvariantV1::TranscriptMessageCounts(&self.parameters)
            }
            InvariantKind::TranscriptAssistantText => {
                DecodedInvariantV1::TranscriptAssistantText(&self.parameters)
            }
            InvariantKind::TranscriptFunctionResult => {
                DecodedInvariantV1::TranscriptFunctionResult(&self.parameters)
            }
            InvariantKind::TranscriptCallsClosed => {
                DecodedInvariantV1::TranscriptCallsClosed(&self.parameters)
            }
            InvariantKind::TranscriptNoDuplicates => {
                DecodedInvariantV1::TranscriptNoDuplicates(&self.parameters)
            }
            InvariantKind::StatusTerminal => DecodedInvariantV1::StatusTerminal(&self.parameters),
            InvariantKind::LifecycleCompletedOnce => {
                DecodedInvariantV1::LifecycleCompletedOnce(&self.parameters)
            }
            InvariantKind::RouterGenerationsConsumed => {
                DecodedInvariantV1::RouterGenerationsConsumed(&self.parameters)
            }
            InvariantKind::TargetCalls => DecodedInvariantV1::TargetCalls(&self.parameters),
        };
        Ok(decoded)
    }

    fn typed(kind: InvariantKind, parameters: impl Serialize) -> Self {
        let parameters = serde_json::to_value(parameters)
            .expect("typed invariant parameters serialize")
            .as_object()
            .cloned()
            .expect("typed invariant parameters are objects");
        Self {
            id: kind.as_str().to_string(),
            parameters,
        }
    }
}
