//! Function registrations for `iii-directory` (formerly `skills` / `engine-catalog`).
//!
//! All public functions sit under a single `directory::*` namespace,
//! split into four sub-namespaces:
//!
//!   * `directory::skills::*` / `directory::prompts::*` — filesystem-backed
//!     reads + downloads. Plain JSON shapes; no envelope or templating.
//!   * `directory::engine::*` — read-side enrichment over engine
//!     introspection (`engine::functions::list`, `engine::workers::list`,
//!     `engine::trigger-types::list`, `engine::triggers::list`).
//!   * `directory::registry::*` — HTTP proxy over the workers registry
//!     (`api.workers.iii.dev`) for worker listing + per-worker metadata.

pub mod directory;
pub mod download;
pub mod prompts;
pub mod registry;
pub mod skills;

use std::sync::Arc;

use iii_sdk::III;

use crate::config::SkillsConfig;
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

/// Register every function the worker exposes against `iii`.
pub fn register_all(
    iii: &Arc<III>,
    cfg: &Arc<SkillsConfig>,
    trigger_types: &RegisteredTriggerTypes,
) {
    skills::register(iii, cfg);
    prompts::register(iii, cfg);
    let subs = Subscribers::from(trigger_types);
    download::register(iii, cfg, &subs);
    directory::register(iii, cfg);
    registry::register(iii, cfg);
    tracing::info!(
        "iii-directory registered 2 directory::skills::* (list + get), \
         2 directory::prompts::* (list + get), 1 directory::skills::download, \
         8 directory::engine::* and 2 directory::registry::workers::* functions"
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
