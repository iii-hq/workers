//! Function registrations for `skills`. Each submodule registers
//! the public CRUD API plus the internal RPC that the `mcp` worker
//! calls when serving MCP `resources/*` / `prompts/*` methods.

pub mod prompts;
pub mod skills;

use std::sync::Arc;

use iii_sdk::III;

use crate::config::SkillsConfig;
use crate::trigger_types::RegisteredTriggerTypes;

/// Register every `skills::*` and `prompts::*` function handler
/// against `iii`. Wires the skills fan-out channel to `trigger_types.skills`
/// and the prompts fan-out channel to `trigger_types.prompts`.
pub fn register_all(
    iii: &Arc<III>,
    cfg: &Arc<SkillsConfig>,
    trigger_types: &RegisteredTriggerTypes,
) {
    skills::register(iii, cfg, &trigger_types.skills);
    prompts::register(iii, cfg, &trigger_types.prompts);
    tracing::info!("skills registered 5 skills::* and 5 prompts::* functions");
}
