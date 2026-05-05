//! `subagent` — wraps `run::start_and_wait` for nested durable sessions.
//!
//! Renamed from `shell-subagent`; nothing about this worker actually
//! involves a shell — it spawns child agent sessions and awaits results.

pub mod register;
pub mod start;

pub use register::register_with_iii;
