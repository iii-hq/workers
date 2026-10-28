//! Engine-backed test bootstrap. Used by unit tests and `tests/integration.rs`.

pub mod engine;

pub use engine::{
    boot, call, engine_bin, harness_bin, hook_input, log_push, log_snapshot, require_engine,
    settle, spawn_engine, spawn_engine_with, state_get, state_set, wait_for, with_stack,
    with_stack_using, BootOpts, CallLog, Engine, EngineOpts, TestStack,
};
