//! The embedded identity prompts. `DEFAULT` is the top-level orchestrator
//! fallback (engine-grounded capability ladder); provider-specific variants
//! live in each provider worker and are served via `router::system_prompt::get`.
//! `SUBAGENT` is the minimal identity EVERY spawned child gets (never the
//! orchestrator prompt): do the one task, write the named state destination,
//! stop — no spawning, no triggers, no workflow knowledge.

pub const DEFAULT: &str = include_str!("../../prompts/default.txt");
pub const SUBAGENT: &str = include_str!("../../prompts/subagent.txt");
