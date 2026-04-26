//! Library entry-point for `iii-a2a`.
//!
//! Exposes `handler` and `types` so integration tests under `a2a/tests/`
//! can reach `build_agent_card` and the agent-card structs without going
//! through the binary.
pub mod handler;
pub mod streaming;
pub mod types;
