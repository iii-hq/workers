//! Wire contracts (README § Cross-cutting contracts) plus harness-internal
//! loop bookkeeping. The message/content/event/model types are wire-identical
//! to `session-manager` and `llm-router` so transcripts and stream frames
//! round-trip across the bus untouched.

pub mod content;
pub mod event;
pub mod message;
pub mod model;
pub mod output;
pub mod turn;

pub use content::ContentBlock;
pub use event::{AssistantMessageEvent, ErrorKind, StopReason, Usage};
pub use message::{
    empty_assistant, AgentMessage, AssistantMessage, AssistantRoleTag, CustomMessage,
    CustomRoleTag, FunctionResultMessage, FunctionResultRoleTag, UserMessage, UserRoleTag,
};
pub use model::{AgentFunction, Model, Pricing, ReasoningEffort, ThinkingLevel};
pub use output::OutputContract;
pub use turn::{
    CallCheckpoint, CallState, ExposeMode, FunctionPolicy, IdemRecord, ParentLink, TurnOptions,
    TurnRecord, TurnStatus,
};
