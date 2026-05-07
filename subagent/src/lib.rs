//! `subagent` — wraps `run::start_and_wait` for nested durable sessions.
//!
//! Renamed from `shell-subagent`; nothing about this worker actually
//! involves a shell — it spawns child agent sessions and awaits results.

pub const SKILL_ID: &str = "subagent";
pub const SKILL_MD: &str = include_str!("../skill.md");
pub const SUB_SKILLS: &[(&str, &str)] = &[("subagent/start", include_str!("../skills/start.md"))];

pub mod config;
pub mod manifest;
pub mod register;
pub mod start;

pub use register::register_with_iii;
