//! Shared data types for the harness agent loop.
//!
//! Pure data shapes. No I/O. No async. All types implement [`serde::Serialize`]
//! and [`serde::Deserialize`] for transport across the iii bus.

mod agent_event;
mod agent_message;
mod content;
mod function;
mod stream_event;
mod thinking;

pub use agent_event::{AgentEvent, ApprovalDecision, Denial};
pub use agent_message::{
    AgentContext, AgentMessage, AgentSessionState, AssistantMessage, CustomMessage,
    FunctionResultMessage, UserMessage,
};
pub use content::{ContentBlock, ImageContent, TextContent};
pub use function::{
    AgentFunction, CacheRetention, ExecutionMode, FinalizedFunctionCall, FunctionCall,
    FunctionResult, PreparedFunctionCall, Transport,
};
pub use stream_event::{AssistantMessageEvent, ErrorKind, StopReason, Usage};
pub use thinking::{TextPhase, TextSignature, ThinkingBudgets, ThinkingLevel};
