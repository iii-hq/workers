// Library crate exposes module surface for integration tests.
// The binary at `src/main.rs` consumes these via `iii_mcp::*` so the tests
// can hit the same code paths a deployed binary uses.

pub mod handler;
pub mod prompts;
pub mod spec;
pub mod transport;
pub mod worker_manager;
