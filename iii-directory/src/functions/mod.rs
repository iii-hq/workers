//! Function registrations for `iii-directory` (formerly `skills` / `engine-catalog`).
//!
//! Public functions sit under a single `directory::*` namespace with five
//! surfaces:
//!
//!   * `directory::search_functions` — compact installed and installable
//!     function candidates.
//!   * `directory::skills::*` — filesystem-backed reads, writes, and downloads.
//!   * `directory::system-prompts::*` — filesystem-backed identity prompts.
//!   * `directory::agents::*` — filesystem-backed reusable agent profiles.
//!   * `directory::registry::*` — workers registry listing and metadata.
//!
//! Engine introspection is native, with one narrow wrapper retained for
//! restricted callers: `directory::engine::functions::info`.

pub mod agents;
pub mod download;
pub mod engine_fn;
pub mod error;
pub mod prompts;
pub mod registry;
pub mod search;
pub mod search_index;
#[cfg(test)]
mod search_relevance;
#[doc(hidden)]
pub mod search_semantic;
pub mod skills;
pub mod update;

use std::sync::Arc;

use iii_sdk::IIIClient;

use crate::config::{SharedConfig, SkillsConfig};
use crate::fs_source::{self, SourceKind};
use crate::trigger_types::{RegisteredTriggerTypes, SubscriberSet};

/// Subscriber sets passed into the write modules (`download`, `update`,
/// `agents`) so they can fan out the matching `directory::*::on-change`
/// after a successful write.
#[derive(Clone)]
pub struct Subscribers {
    pub skills: SubscriberSet,
    pub system_prompts: SubscriberSet,
    pub agents: SubscriberSet,
}

impl From<&RegisteredTriggerTypes> for Subscribers {
    fn from(t: &RegisteredTriggerTypes) -> Self {
        Self {
            skills: t.skills.clone(),
            system_prompts: t.system_prompts.clone(),
            agents: t.agents.clone(),
        }
    }
}

pub fn register_all(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    trigger_types: &RegisteredTriggerTypes,
) {
    let cache = std::sync::Arc::new(skills::RegisteredWorkersCache::new(
        cfg.load().registry_cache_ttl_ms,
    ));
    skills::register_with_cache(iii, cfg, &cache);
    prompts::register(iii, cfg);
    let subs = Subscribers::from(trigger_types);
    download::register(iii, cfg, &subs);
    update::register(iii, cfg, &subs, &cache);
    agents::register(iii, cfg, &subs, &cache);
    registry::register(iii, cfg);
    engine_fn::register(iii);
}

pub fn register_all_with_cache(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    trigger_types: &RegisteredTriggerTypes,
    cache: &std::sync::Arc<skills::RegisteredWorkersCache>,
    registry_cache: registry::RegistryCache,
) {
    skills::register_with_cache(iii, cfg, cache);
    prompts::register(iii, cfg);
    let subs = Subscribers::from(trigger_types);
    download::register(iii, cfg, &subs);
    update::register(iii, cfg, &subs, cache);
    agents::register(iii, cfg, &subs, cache);
    registry::register_with_cache(iii, cfg, registry_cache);
    engine_fn::register(iii);
}

/// One-shot diagnostic of the configured `skills_folder`. Called from
/// `main` after `load_config` so misconfigured layouts surface in the
/// boot log instead of failing silently at first read.
pub fn log_fs_health(cfg: &SkillsConfig) {
    let folder = cfg.resolved_skills_folder();
    let (skills, skill_skipped) = fs_source::scan_skills(&folder);
    let (system_prompts, system_prompt_skipped) = fs_source::scan_system_prompts(&folder);
    let agents_folder = cfg.resolved_agents_folder();
    let (agent_profiles, agent_skipped) =
        fs_source::scan_agents_merged(&cfg.resolved_agents_roots());
    let mut agents_skills = Vec::new();
    let mut agents_skipped = Vec::new();
    for root in cfg.resolved_agents_skills_roots() {
        let (skills, skipped) = fs_source::scan_agents_skills(&root);
        agents_skills.extend(skills);
        agents_skipped.extend(skipped);
    }

    for s in &skills {
        tracing::info!(
            id = %s.id,
            path = %s.abs_path.display(),
            "loaded fs-backed skill"
        );
    }
    for s in &agents_skills {
        tracing::info!(
            id = %s.id,
            path = %s.abs_path.display(),
            "loaded system-installed agents skill"
        );
    }
    for p in &system_prompts {
        tracing::info!(
            name = %p.name,
            path = %p.abs_path.display(),
            "loaded fs-backed system prompt"
        );
    }
    for a in &agent_profiles {
        tracing::info!(
            id = %a.name,
            path = %a.abs_path.display(),
            "loaded fs-backed agent profile"
        );
    }

    let total_skipped = skill_skipped.len()
        + system_prompt_skipped.len()
        + agent_skipped.len()
        + agents_skipped.len();
    for s in skill_skipped
        .iter()
        .chain(system_prompt_skipped.iter())
        .chain(agent_skipped.iter())
        .chain(agents_skipped.iter())
    {
        let kind = match s.kind {
            SourceKind::Skill => "skill",
            SourceKind::SystemPrompt => "system prompt",
            SourceKind::Agent => "agent profile",
        };
        tracing::warn!(
            kind,
            path = %s.path.display(),
            reason = %s.reason,
            "skipped fs entry"
        );
    }

    tracing::info!(
        skills = skills.len(),
        agents_skills = agents_skills.len(),
        system_prompts = system_prompts.len(),
        agent_profiles = agent_profiles.len(),
        skipped = total_skipped,
        skills_folder = %folder.display(),
        agents_folder = %agents_folder.display(),
        "fs source scan complete"
    );
}
