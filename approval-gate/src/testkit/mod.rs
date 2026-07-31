//! Engine-backed test bootstrap. Used by unit tests and `tests/integration.rs`.

pub mod engine;

pub use engine::{
    boot, boot_harness_only, call, engine_bin, engine_test_guard, hook_input, log_push,
    log_snapshot, require_engine, settle, spawn_engine, state_get, state_set, wait_for, with_stack,
    BootOpts, CallLog, Engine, HarnessOnlyStack, TestStack,
};
