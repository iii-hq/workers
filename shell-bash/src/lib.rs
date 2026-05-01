//! `shell-bash` — `shell::bash::*` over `sandbox::exec`.
//!
//! Fail-closed: there is no host-shell fallback.

pub mod detect_clis;
pub mod exec;
pub mod register;
pub mod which;

pub use register::register_with_iii;
