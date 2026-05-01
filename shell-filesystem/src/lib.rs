//! `shell-filesystem` — wraps `sandbox::fs::*` triggers as
//! `shell::filesystem::<op>` functions discoverable by the
//! turn orchestrator.

pub mod ops;
pub mod register;

pub use register::register_with_iii;
