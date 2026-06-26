//! Function registrations for `iii-directory` (formerly `skills` / `engine-catalog`).
//!
//! All public functions sit under a single `directory::*` namespace,
//! split into three sub-namespaces:
//!
//!   * `directory::skills::*` / `directory::prompts::*` — filesystem-backed
//!     reads + downloads. Plain JSON shapes; no envelope or templating.
//!   * `directory::registry::*` — HTTP proxy over the workers registry
//!     (`api.workers.iii.dev`) for worker listing + per-worker metadata.
//!
//! Engine introspection (functions / triggers / workers / registered
//! triggers) is no longer wrapped here — callers should invoke the
//! native ids directly: `engine::functions::list`,
//! `engine::trigger-types::list`, `engine::triggers::list`,
//! `engine::workers::list`. See the harness `iii` skill for the
//! recommended composition patterns.

pub mod download;
pub mod engine_fn;
pub mod error;
pub mod prompts;
pub mod registry;
pub mod skills;

use std::sync::Arc;

use iii_sdk::IIIClient;

use crate::config::{SharedConfig, SkillsConfig};
use crate::fs_source::{self, SourceKind};
use crate::trigger_types::{RegisteredTriggerTypes, SubscriberSet};

/// Pair of subscriber sets passed into `download::register` so the
/// download function can fan out to both `directory::skills::on-change`
/// and `directory::prompts::on-change` after a successful pull.
pub struct Subscribers {
    pub skills: SubscriberSet,
    pub prompts: SubscriberSet,
}

impl From<&RegisteredTriggerTypes> for Subscribers {
    fn from(t: &RegisteredTriggerTypes) -> Self {
        Self {
            skills: t.skills.clone(),
            prompts: t.prompts.clone(),
        }
    }
}

pub fn register_all(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    trigger_types: &RegisteredTriggerTypes,
) {
    skills::register(iii, cfg);
    prompts::register(iii, cfg);
    let subs = Subscribers::from(trigger_types);
    download::register(iii, cfg, &subs);
    registry::register(iii, cfg);
    engine_fn::register(iii);
    tracing::info!(
        "iii-directory registered 3 directory::skills::* (list + get + index), \
         2 directory::prompts::* (list + get), 1 directory::skills::download, \
         2 directory::registry::workers::*, and 1 directory::engine::functions::info"
    );
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
    registry::register_with_cache(iii, cfg, registry_cache);
    engine_fn::register(iii);
    tracing::info!(
        "iii-directory registered 3 directory::skills::* (list + get + index), \
         2 directory::prompts::* (list + get), 1 directory::skills::download, \
         2 directory::registry::workers::*, and 1 directory::engine::functions::info"
    );
}

/// One-shot diagnostic of the configured `skills_folder`. Called from
/// `main` after `load_config` so misconfigured layouts surface in the
/// boot log instead of failing silently at first read.
pub fn log_fs_health(cfg: &SkillsConfig) {
    let folder = cfg.resolved_skills_folder();
    let (skills, skill_skipped) = fs_source::scan_skills(&folder);
    let (prompts, prompt_skipped) = fs_source::scan_prompts(&folder);

    for s in &skills {
        tracing::info!(
            id = %s.id,
            path = %s.abs_path.display(),
            "loaded fs-backed skill"
        );
    }
    for p in &prompts {
        tracing::info!(
            name = %p.name,
            path = %p.abs_path.display(),
            "loaded fs-backed prompt"
        );
    }

    let total_skipped = skill_skipped.len() + prompt_skipped.len();
    for s in skill_skipped.iter().chain(prompt_skipped.iter()) {
        let kind = match s.kind {
            SourceKind::Skill => "skill",
            SourceKind::Prompt => "prompt",
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
        prompts = prompts.len(),
        skipped = total_skipped,
        skills_folder = %folder.display(),
        "fs source scan complete"
    );
}
