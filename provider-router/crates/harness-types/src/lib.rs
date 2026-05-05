//! Shared data types for the harness agent loop.
//!
//! Pure data shapes. No I/O. No async. All types implement [`serde::Serialize`]
//! and [`serde::Deserialize`] for transport across the iii bus.

mod agent_event;
mod agent_message;
mod content;
mod stream_event;
mod thinking;
mod tool;

pub use agent_event::AgentEvent;
pub use agent_message::{
    AgentContext, AgentMessage, AgentSessionState, AssistantMessage, CustomMessage,
    ToolResultMessage, UserMessage,
};
pub use content::{ContentBlock, ImageContent, TextContent};
pub use stream_event::{AssistantMessageEvent, ErrorKind, StopReason, Usage};
pub use thinking::{TextPhase, TextSignature, ThinkingBudgets, ThinkingLevel};
pub use tool::{
    AgentTool, CacheRetention, ExecutionMode, FinalizedToolCall, PreparedToolCall, ToolCall,
    ToolResult, Transport,
};
