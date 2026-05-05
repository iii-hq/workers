//! Harness runtime: iii-bus surface for harness primitives, shells, and
//! the agent::stream_assistant provider router.

pub mod register;
pub mod resume;
pub mod runtime;

pub use register::{
    register_with_iii, EVENTS_STREAM, HOOK_REPLY_STREAM, STATE_SCOPE, TOPIC_AFTER, TOPIC_BEFORE,
};
pub use runtime::EventSink;
