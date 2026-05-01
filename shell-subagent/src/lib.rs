//! `shell-subagent` — wraps `run::start_and_wait` for nested durable sessions.

pub mod register;
pub mod start;

pub use register::register_with_iii;
