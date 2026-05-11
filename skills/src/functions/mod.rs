//! Function registrations for `skills`. Each submodule registers
//! the public CRUD API plus the internal RPC that the `mcp` worker
//! calls when serving MCP `resources/*` / `prompts/*` methods.

pub mod prompts;
pub mod roots;
pub mod skills;

use std::sync::Arc;

use iii_sdk::III;

use crate::config::SkillsConfig;
use crate::fs_source::{self, SourceKind};
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
    roots::register(iii);
    tracing::info!("skills registered 7 skills::*, 1 skill::*, 3 skills::roots::* and 5 prompts::* functions");
}

/// One-shot diagnostic of the configured filesystem-backed sources.
/// Called from `main` after `load_config` so misconfigured globs surface
/// in the boot log instead of failing silently at first read.
///
/// Logs:
///
/// - One `info!` per successfully loaded skill / prompt (id, abs_path).
/// - One `warn!` per skipped file (path + reason).
/// - One `info!` summary line ("loaded N skills, M prompts; K skipped").
///
/// Note: this scan is independent of the per-call re-globbing the
/// runtime does on every list/read; the file system stays the source
/// of truth throughout the lifetime of the worker.
pub fn log_fs_health(cfg: &SkillsConfig) {
    let base = cfg.glob_base_dir();
    let (skills, skill_skipped) = fs_source::expand_skill_globs(&base, &cfg.skills);
    let (prompts, prompt_skipped) = fs_source::expand_prompt_globs(&base, &cfg.prompts);

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
        base_dir = %base.display(),
        "fs source scan complete"
    );
}
