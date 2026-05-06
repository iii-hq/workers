//! Provider router: iii-bus surface for `router::stream_assistant`
//! (provider routing with optional `router::decide` indirection from
//! llm-router) plus `router::abort` and the `router::push_steering` /
//! `push_followup` inbox helpers.
//!
//! Renamed from `harness-runtime` to reflect actual responsibility: the
//! turn-execution loop lives in `turn-orchestrator`, not here.

pub mod register;
pub mod resume;
pub mod runtime;

pub use register::{
    register_with_iii, EVENTS_STREAM, HOOK_REPLY_STREAM, STATE_SCOPE, TOPIC_AFTER, TOPIC_BEFORE,
};
pub use runtime::EventSink;
