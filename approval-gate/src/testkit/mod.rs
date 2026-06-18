//! Engine-backed test bootstrap. Used by unit tests and `tests/integration.rs`.

pub mod engine;

pub use engine::{
    boot, call, engine_bin, hook_input, log_push, log_snapshot, require_engine, settle,
    spawn_engine, state_get, state_set, wait_for, with_stack, BootOpts, CallLog, Engine, TestStack,
};
