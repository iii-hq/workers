//! Where kit state lives on disk.
//!
//! * `kits.lock` sits next to the project's compose file and is committed.
//!   The compose file is `III_COMPOSE_FILE` when Compose started this worker
//!   (Compose sets it to the canonical path of the `worker-compose.yaml` that
//!   declares the worker); otherwise the project root is `III_COMPOSE_DIR`, or
//!   the process working directory for a standalone worker, and the compose
//!   file is `worker-compose.yaml` there.
//! * Pending plans live under `<project root>/.iii/directory/kit-plans/`,
//!   one `<plan_id>.json` each — local state, never committed.
//! * The last update check is cached at
//!   `<project root>/.iii/directory/kit-updates.json`.
//!
//! Install paths (the keys of `kits.lock` `files` and of a plan's `files`)
//! are logical: `agents/<id>.md` lives under `agents_folder`, and
//! `skills/<handle>/<kit>/<path>` under `skills_folder`. With the default
//! configuration both read as project-relative paths.

use std::path::{Path, PathBuf};

use crate::config::SkillsConfig;

/// File name of the kit lock, next to the compose file.
pub const KITS_LOCK_FILE: &str = "kits.lock";
/// Default compose file name in a project root.
pub const COMPOSE_FILE: &str = "worker-compose.yaml";

/// Absolute path of the project's compose file.
pub fn compose_file() -> PathBuf {
    match std::env::var_os("III_COMPOSE_FILE").filter(|v| !v.is_empty()) {
        Some(file) => PathBuf::from(file),
        None => iii_worker_paths::project_path(COMPOSE_FILE),
    }
}

/// Directory that holds the compose file — the project root for kit state.
pub fn project_root() -> PathBuf {
    let file = compose_file();
    file.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| iii_worker_paths::project_path("."))
}

/// `kits.lock`, next to the compose file.
pub fn kits_lock() -> PathBuf {
    project_root().join(KITS_LOCK_FILE)
}

/// The compose lock (`worker-compose.lock`) Compose writes next to its file.
pub fn compose_lock() -> PathBuf {
    compose_file().with_extension("lock")
}

/// Directory of pending kit plans.
pub fn plans_dir() -> PathBuf {
    project_root()
        .join(".iii")
        .join("directory")
        .join("kit-plans")
}

/// Cached result of the last `directory::kits::check-updates`.
pub fn updates_cache() -> PathBuf {
    project_root()
        .join(".iii")
        .join("directory")
        .join("kit-updates.json")
}

/// What a logical install path points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallTarget {
    /// `agents/<id>.md`.
    Agent { id: String },
    /// `skills/<rest>` (rest includes `<handle>/<kit>/`).
    Skill { rest: String },
}

/// Classify a logical install path. `None` for anything outside the two
/// families or with an unsafe segment.
pub fn classify_install_path(path: &str) -> Option<InstallTarget> {
    let rel = crate::sources::validate_relative_path(path).ok()?;
    let mut parts = rel.iter().map(|p| p.to_str());
    match parts.next()?? {
        "agents" => {
            let file = parts.next()??;
            if parts.next().is_some() {
                return None;
            }
            let id = file.strip_suffix(".md")?;
            crate::functions::prompts::validate_name(id).ok()?;
            Some(InstallTarget::Agent { id: id.to_string() })
        }
        "skills" => {
            let rest: Vec<&str> = parts.collect::<Option<Vec<_>>>()?;
            if rest.len() < 3 || !rest.last()?.ends_with(".md") {
                return None;
            }
            Some(InstallTarget::Skill {
                rest: rest.join("/"),
            })
        }
        _ => None,
    }
}

/// Absolute file for a logical install path under the configured roots.
pub fn resolve_install_path(cfg: &SkillsConfig, path: &str) -> Option<PathBuf> {
    match classify_install_path(path)? {
        InstallTarget::Agent { id } => Some(cfg.resolved_agents_folder().join(format!("{id}.md"))),
        InstallTarget::Skill { rest } => Some(cfg.resolved_skills_folder().join(rest)),
    }
}

/// Map a kit SOURCE path (`agents/<id>.md`, `skills/<path>.md`) to its
/// logical install path for kit `<handle>/<name>`.
pub fn install_path_for_source(kit: &str, source: &str) -> Option<String> {
    if let Some(file) = source.strip_prefix("agents/") {
        if file.contains('/') {
            return None;
        }
        return Some(source.to_string());
    }
    let rest = source.strip_prefix("skills/")?;
    Some(format!("skills/{kit}/{rest}"))
}

/// Inverse of [`install_path_for_source`].
pub fn source_for_install_path(kit: &str, path: &str) -> Option<String> {
    if path.starts_with("agents/") {
        return Some(path.to_string());
    }
    let rest = path.strip_prefix(&format!("skills/{kit}/"))?;
    Some(format!("skills/{rest}"))
}

/// Skill id for a kit source path: `<handle>/<kit>/<path without .md>`,
/// with a trailing `SKILL.md` / `SKILLS.md` / `index.md` mapping to `index`.
pub fn skill_id_for_source(kit: &str, source: &str) -> Option<String> {
    let rest = source.strip_prefix("skills/")?.strip_suffix(".md")?;
    let rest = match rest.rsplit_once('/') {
        Some((dir, "SKILL" | "SKILLS")) => format!("{dir}/index"),
        None if rest == "SKILL" || rest == "SKILLS" => "index".to_string(),
        _ => rest.to_string(),
    };
    Some(format!("{kit}/{rest}"))
}

/// Agent id for `agents/<id>.md`.
pub fn agent_id_for_path(path: &str) -> Option<String> {
    match classify_install_path(path)? {
        InstallTarget::Agent { id } => Some(id),
        InstallTarget::Skill { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_paths_round_trip_between_source_and_disk() {
        let kit = "acme/kanban-team";
        assert_eq!(
            install_path_for_source(kit, "agents/planner.md").as_deref(),
            Some("agents/planner.md")
        );
        assert_eq!(
            install_path_for_source(kit, "skills/tickets/flow.md").as_deref(),
            Some("skills/acme/kanban-team/tickets/flow.md")
        );
        assert_eq!(
            source_for_install_path(kit, "skills/acme/kanban-team/tickets/flow.md").as_deref(),
            Some("skills/tickets/flow.md")
        );
        assert_eq!(install_path_for_source(kit, "agents/x/y.md"), None);
        assert_eq!(install_path_for_source(kit, "README.md"), None);
    }

    #[test]
    fn classify_rejects_traversal_and_nested_agents() {
        assert_eq!(
            classify_install_path("agents/reviewer.md"),
            Some(InstallTarget::Agent {
                id: "reviewer".into()
            })
        );
        assert_eq!(classify_install_path("agents/../x.md"), None);
        assert_eq!(classify_install_path("agents/a/b.md"), None);
        assert_eq!(classify_install_path("skills/acme/x.md"), None);
        assert_eq!(
            classify_install_path("skills/acme/kit/a/b.md"),
            Some(InstallTarget::Skill {
                rest: "acme/kit/a/b.md".into()
            })
        );
        assert_eq!(classify_install_path("/etc/passwd"), None);
    }

    #[test]
    fn skill_ids_follow_the_directory_convention() {
        let kit = "acme/kanban-team";
        assert_eq!(
            skill_id_for_source(kit, "skills/tickets/flow.md").as_deref(),
            Some("acme/kanban-team/tickets/flow")
        );
        assert_eq!(
            skill_id_for_source(kit, "skills/SKILL.md").as_deref(),
            Some("acme/kanban-team/index")
        );
        assert_eq!(
            skill_id_for_source(kit, "skills/tickets/SKILL.md").as_deref(),
            Some("acme/kanban-team/tickets/index")
        );
    }
}
