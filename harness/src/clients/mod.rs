//! Typed clients for the workers the loop wires together. Each is a thin
//! wrapper over `iii.trigger(TriggerRequest { .. })`; `session-manager` and
//! `llm-router` and `context-manager` are required on the generation path;
//! context budgeting fails closed rather than degrading to raw history.

pub mod context;
pub mod engine;
pub mod router;
pub mod session;

pub use context::{AssembleOutput, AssembleParams, ContextClient};
pub use engine::{EngineClient, FunctionDescriptor};
pub use router::{ChatOutcome, ChatParams, RouterClient, StreamSink};
pub use session::{LoadedCustom, LoadedEntry, SessionClient};
