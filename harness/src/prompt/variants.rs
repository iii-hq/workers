//! The embedded identity prompts. `DEFAULT` is the top-level orchestrator
//! identity (engine-grounded capability ladder).
//! `SUBAGENT` is the minimal identity EVERY spawned child gets (never the
//! orchestrator prompt): do the one task, write the named state destination,
//! stop — no spawning, no triggers, no workflow knowledge.

pub const DEFAULT: &str = include_str!("../../prompts/default.txt");
pub const SUBAGENT: &str = include_str!("../../prompts/subagent.txt");
