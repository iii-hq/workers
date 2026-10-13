//! Kit plans — the contract between `directory::download-kit` /
//! `plan-update` / `remove`, `directory::kits::apply`, the Kits page and the
//! chat cards.
//!
//! A plan says, before anything is written, what an install, update or
//! removal would do to this project: every file with its change and local
//! state, every collision with a file someone else owns, the three-way
//! merge of files the user edited, the workers Compose would add or move,
//! the capabilities the kit's profiles gain, and what blocks the apply.
//! File bodies are not part of the plan itself: they ride in
//! [`PlanContents`], stored next to it and served by `directory::kits::plan`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::compose::{declared_workers, DeclaredWorker};
use super::kitref::{bump_kind, is_major_jump, satisfies};
use super::lock::{local_sha, sha256_hex, FileState, KitsLock, LockedKit};
use super::merge::{three_way, MergeStatus};
use super::origin::{OriginIndex, SourceKind};
use super::paths::{
    agent_id_for_path, install_path_for_source, skill_id_for_source, source_for_install_path,
};
use super::registry::{
    AgentEntry, Author, Capabilities, Deprecation, FunctionAdded, KitDetail, KitFunction,
    KitRegistry, ModelChanged, SkillEntry,
};
use crate::config::SkillsConfig;

/// How long a pending plan stays valid.
pub const PLAN_TTL_HOURS: i64 = 24;

/// Profiles bundled with the worker (`crate::bundled`).
pub const BUILTIN_AGENTS: [&str; 3] = ["default", "iii", "iii-minimal"];

/// Where kit operations read and write — resolved once from the config so
/// tests can point everything at a temporary directory.
#[derive(Debug, Clone)]
pub struct KitsEnv {
    pub agents_folder: PathBuf,
    /// Lower-precedence agent roots (`~/.iii/agents`).
    pub global_agent_roots: Vec<PathBuf>,
    pub skills_folder: PathBuf,
    pub lock_path: PathBuf,
    pub compose_file: PathBuf,
    pub plans_dir: PathBuf,
    pub updates_cache: PathBuf,
    pub registry_base: String,
    pub registry_web: Option<String>,
    /// Count downloads as CI (`CI` is set in the environment).
    pub ci: bool,
}

impl KitsEnv {
    pub fn from_config(cfg: &SkillsConfig) -> Self {
        let roots = cfg.resolved_agents_roots();
        Self {
            agents_folder: cfg.resolved_agents_folder(),
            global_agent_roots: roots.into_iter().skip(1).collect(),
            skills_folder: cfg.resolved_skills_folder(),
            lock_path: super::paths::kits_lock(),
            compose_file: super::paths::compose_file(),
            plans_dir: super::paths::plans_dir(),
            updates_cache: super::paths::updates_cache(),
            registry_base: cfg.registry_base().to_string(),
            registry_web: cfg.registry_web_url.clone(),
            ci: std::env::var_os("CI").is_some_and(|v| !v.is_empty() && v != "false" && v != "0"),
        }
    }

    /// Absolute file behind a logical install path.
    pub fn abs(&self, install_path: &str) -> Option<PathBuf> {
        match super::paths::classify_install_path(install_path)? {
            super::paths::InstallTarget::Agent { id } => {
                Some(self.agents_folder.join(format!("{id}.md")))
            }
            super::paths::InstallTarget::Skill { rest } => Some(self.skills_folder.join(rest)),
        }
    }

    pub fn read_lock(&self) -> Result<KitsLock, String> {
        KitsLock::read(&self.lock_path)
    }

    pub fn kit_web_url(&self, kit: &str) -> String {
        super::registry::kit_web_url(&self.registry_base, self.registry_web.as_deref(), kit)
    }

    pub fn worker_web_url(&self, worker: &str) -> String {
        super::registry::worker_web_url(&self.registry_base, self.registry_web.as_deref(), worker)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanKind {
    Install,
    Update,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Agent,
    Skill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Added,
    Modified,
    Removed,
    /// The lock keeps the user's choice to skip this file; the kit's
    /// version changed and is offered again.
    Kept,
}

/// What the file is on disk right now, relative to the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LocalState {
    /// Nothing at the path.
    Absent,
    /// Matches the kit's base (or, on install, the kit's content).
    Intact,
    /// Differs from the base the kit installed.
    Edited,
    /// The lock lists it but the file was deleted.
    Missing,
    /// Someone else's file sits at the path.
    Occupied,
}

/// A per-file choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Write the kit's file over what is there.
    Overwrite,
    /// Keep what is there; the lock marks the kit's file `skipped`.
    Keep,
    /// Take the kit's new version (drops local edits).
    Kit,
    /// Keep the local file as it is.
    Mine,
    /// Write the merge result (the plan's, or `content` from the caller).
    Merged,
    /// Delete the file.
    Remove,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Collision {
    /// Who owns the file at the path today.
    pub owner: SourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// sha256 of the file there now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The owner's copy was edited locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<bool>,
}

/// Changed frontmatter of a profile between two kit versions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentChanges {
    /// Scalar fields that changed: `field → [from, to]`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, (Option<String>, Option<String>)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub functions_added: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub functions_removed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills_added: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills_removed: Vec<String>,
}

impl AgentChanges {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
            && self.functions_added.is_empty()
            && self.functions_removed.is_empty()
            && self.skills_added.is_empty()
            && self.skills_removed.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlanFile {
    /// Logical install path (`agents/<id>.md`, `skills/<handle>/<kit>/…`).
    pub path: String,
    /// Path inside the kit (`agents/<id>.md`, `skills/…`).
    pub source: String,
    pub kind: FileKind,
    /// Agent profile id, or skill id.
    pub id: String,
    pub change: Change,
    pub local: LocalState,
    /// Content hash the kit installed (the merge base).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Content hash of the kit's version this plan installs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theirs: Option<String>,
    /// Content hash of the local file when the plan was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ours: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collision: Option<Collision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<MergeStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicts: Option<usize>,
    /// Applied when the caller decides nothing; `null` = a decision is required.
    pub default: Option<Decision>,
    /// Choices the apply accepts for this file.
    pub options: Vec<Decision>,
    /// The lock says the user kept someone else's file here.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
    /// One-line explanation for the review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Kit profile frontmatter (agents; the side this plan installs, or the
    /// removed one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentEntry>,
    /// Frontmatter changes versus the installed version (modified agents).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_changes: Option<AgentChanges>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_changed: Option<bool>,
    /// Skill frontmatter (skills).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<SkillEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkerAction {
    /// Not in the compose file: `compose::add` declares it with the kit's range.
    Add,
    /// Declared, but the installed version is outside the kit's range:
    /// `compose::add` re-declares the range and Compose moves the worker.
    Update,
    /// The range changed and the compose file still declares the kit's old
    /// range; the installed version already satisfies the new one, so only
    /// the declaration moves.
    Redeclare,
    None,
    /// The kit no longer declares it. Never removed unless asked
    /// (`remove_workers`).
    Remove,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlanWorker {
    pub name: String,
    /// Range the kit declares (absent for a worker the kit dropped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    /// Range the installed kit version declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_from: Option<String>,
    /// Version Compose has installed.
    pub installed: Option<String>,
    /// `version:` selector in the compose file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    pub action: WorkerAction,
    /// Version the range resolves to in the registry today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Worker type (`binary`, `image`, …).
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The operator's `path://` build; kits never manage it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub path: bool,
    /// Other installed kits that declare this worker.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub used_by: Vec<String>,
    pub registry_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Issue {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanCounts {
    pub agents: usize,
    pub skills: usize,
    pub workers: usize,
    pub functions: usize,
    pub added: usize,
    pub modified: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub collisions: usize,
    pub conflicts: usize,
    /// Files whose `default` is `null`.
    pub decisions_required: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    pub plan_id: String,
    pub kind: PlanKind,
    pub kit: String,
    /// Installed version (update, remove).
    pub from: Option<String>,
    /// Version this plan installs (install, update).
    pub to: Option<String>,
    /// What was asked for (tag, version or range) — stored in the lock.
    pub requested: String,
    /// `major` | `minor` | `patch` | `prerelease` (update).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bump: Option<String>,
    pub major: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    /// The kit's page in the registry web app.
    pub registry_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecation: Option<Deprecation>,
    pub workers: Vec<PlanWorker>,
    pub files: Vec<PlanFile>,
    pub capabilities: Capabilities,
    /// Functions the kit's profiles preload, with the worker that provides each.
    pub functions: Vec<KitFunction>,
    pub warnings: Vec<Issue>,
    pub blocking: Vec<Issue>,
    pub counts: PlanCounts,
    pub created_at: String,
    pub expires_at: String,
    /// Where a human reviews it.
    pub review: String,
}

/// File bodies a plan refers to, stored beside it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlanContents {
    /// Kit blobs by sha256 (theirs and bases).
    #[serde(default)]
    pub blobs: BTreeMap<String, String>,
    /// Local file bodies when the plan was made, by install path.
    #[serde(default)]
    pub ours: BTreeMap<String, String>,
    /// Three-way merge results, by install path.
    #[serde(default)]
    pub merged: BTreeMap<String, String>,
}

pub fn new_plan_id() -> String {
    format!("kp_{}", uuid::Uuid::new_v4().simple())
}

fn now_rfc3339(now: chrono::DateTime<chrono::Utc>) -> String {
    now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub const REVIEW_HINT: &str = "Review it in the ADE (Directory → Kits) or apply it with \
     `iii trigger directory::kits::apply plan_id=<plan_id>`.";

/// Everything a plan builder reads besides the registry.
pub struct LocalView<'a> {
    pub env: &'a KitsEnv,
    pub lock: &'a KitsLock,
    pub origins: OriginIndex,
    pub declared: BTreeMap<String, DeclaredWorker>,
    /// `compose::status` answered.
    pub compose_available: bool,
}

impl<'a> LocalView<'a> {
    pub fn load(env: &'a KitsEnv, lock: &'a KitsLock, compose_available: bool) -> Self {
        Self {
            env,
            lock,
            origins: OriginIndex::load(lock.clone(), &env.skills_folder),
            declared: declared_workers(&env.compose_file),
            compose_available,
        }
    }

    fn read(&self, install_path: &str) -> Option<String> {
        self.env
            .abs(install_path)
            .and_then(|p| std::fs::read_to_string(p).ok())
    }
}

/// The kit's files at one version: install path → (source, sha, content).
fn index_files(
    kit: &str,
    files: &super::registry::KitFiles,
) -> BTreeMap<String, (String, String, String)> {
    let mut out = BTreeMap::new();
    for file in &files.files {
        let Some(path) = install_path_for_source(kit, &file.path) else {
            continue;
        };
        if super::paths::classify_install_path(&path).is_none() {
            continue;
        }
        let sha = sha256_hex(file.content.as_bytes());
        out.insert(path, (file.path.clone(), sha, file.content.clone()));
    }
    out
}

fn kind_of(path: &str) -> FileKind {
    if path.starts_with("agents/") {
        FileKind::Agent
    } else {
        FileKind::Skill
    }
}

fn id_of(kit: &str, path: &str, source: &str) -> String {
    agent_id_for_path(path)
        .or_else(|| skill_id_for_source(kit, source))
        .unwrap_or_else(|| path.to_string())
}

fn agent_entry(detail: &KitDetail, source: &str) -> Option<AgentEntry> {
    detail.agents.iter().find(|a| a.path == source).cloned()
}

fn skill_entry(detail: &KitDetail, source: &str) -> Option<SkillEntry> {
    detail.skills.iter().find(|s| s.path == source).cloned()
}

/// Who already owns a path the kit wants to write (install, or a file an
/// update adds). `None` when the path is free or holds identical content.
fn collision_for(
    view: &LocalView<'_>,
    kit: &str,
    path: &str,
    theirs_sha: &str,
) -> (LocalState, Option<String>, Option<Collision>) {
    let abs = view.env.abs(path);
    let local = abs.as_deref().and_then(local_sha);
    if let Some(sha) = &local {
        if sha == theirs_sha {
            return (LocalState::Intact, local, None);
        }
        let collision = match agent_id_for_path(path) {
            Some(id) => {
                let source = view
                    .origins
                    .classify_project_agent(&id, abs.as_deref().unwrap_or(Path::new("")));
                match source.kind {
                    // The kit owns it already (a reinstall without a lock entry).
                    SourceKind::Kit if source.kit.as_deref() == Some(kit) => None,
                    _ => Some(Collision {
                        owner: source.kind,
                        kit: source.kit,
                        worker: source.worker,
                        version: source.version,
                        sha256: local.clone(),
                        modified: source.modified,
                    }),
                }
            }
            None => Some(Collision {
                owner: SourceKind::Local,
                kit: None,
                worker: None,
                version: None,
                sha256: local.clone(),
                modified: None,
            }),
        };
        return (LocalState::Occupied, local, collision);
    }
    // Nothing in the project root: an agent id may still exist globally or
    // be bundled with the worker, and the project copy would shadow it.
    if let Some(id) = agent_id_for_path(path) {
        for root in &view.env.global_agent_roots {
            let global = root.join(format!("{id}.md"));
            if let Some(sha) = local_sha(&global) {
                return (
                    LocalState::Absent,
                    None,
                    Some(Collision {
                        owner: SourceKind::Global,
                        kit: None,
                        worker: None,
                        version: None,
                        sha256: Some(sha),
                        modified: None,
                    }),
                );
            }
        }
        if BUILTIN_AGENTS.contains(&id.as_str()) {
            return (
                LocalState::Absent,
                None,
                Some(Collision {
                    owner: SourceKind::Builtin,
                    kit: None,
                    worker: None,
                    version: None,
                    sha256: None,
                    modified: None,
                }),
            );
        }
    }
    (LocalState::Absent, None, None)
}

/// Warning for a collision, worded by owner — the local case is the strongest.
fn collision_issue(path: &str, c: &Collision) -> Issue {
    let (code, message) = match c.owner {
        SourceKind::Worker => (
            "agent_overwrite",
            format!(
                "{path} came from worker {}{} and will be replaced{}.",
                c.worker.as_deref().unwrap_or("?"),
                c.version.as_deref().map(|v| format!(" {v}")).unwrap_or_default(),
                if c.modified == Some(true) {
                    " (it was edited in this project)"
                } else {
                    ""
                }
            ),
        ),
        SourceKind::Kit => (
            "agent_overwrite_kit",
            format!(
                "{path} belongs to kit {}{}; replacing it moves the file to this kit.",
                c.kit.as_deref().unwrap_or("?"),
                c.version.as_deref().map(|v| format!(" {v}")).unwrap_or_default()
            ),
        ),
        SourceKind::Local if path.starts_with("agents/") => (
            "agent_overwrite_local",
            format!(
                "{path} was created in this project and does not come from any package; \
                 replacing it loses that profile."
            ),
        ),
        SourceKind::Local => (
            "file_overwrite_local",
            format!("{path} already exists in this project and is not tracked by kits.lock."),
        ),
        SourceKind::Global => (
            "agent_shadows_global",
            format!(
                "{path} shadows your user-global profile of the same id (~/.iii/agents) in this project."
            ),
        ),
        SourceKind::Builtin => (
            "builtin_override",
            format!("{path} replaces a profile built into iii-directory for this whole project."),
        ),
    };
    Issue {
        code: code.to_string(),
        path: Some(path.to_string()),
        message,
    }
}

fn collision_note(c: &Collision) -> String {
    match c.owner {
        SourceKind::Worker => format!(
            "came from worker {}{}",
            c.worker.as_deref().unwrap_or("?"),
            c.version
                .as_deref()
                .map(|v| format!(" {v}"))
                .unwrap_or_default()
        ),
        SourceKind::Kit => format!("belongs to kit {}", c.kit.as_deref().unwrap_or("?")),
        SourceKind::Local => "created in this project".to_string(),
        SourceKind::Global => "user-global profile".to_string(),
        SourceKind::Builtin => "built-in profile".to_string(),
    }
}

/// The plan entry for a file the kit adds (install, or new in an update).
fn added_file(
    view: &LocalView<'_>,
    kit: &str,
    detail: &KitDetail,
    path: &str,
    source: &str,
    theirs: &str,
    contents: &mut PlanContents,
) -> PlanFile {
    let (local, ours, collision) = collision_for(view, kit, path, theirs);
    if let (Some(_), Some(text)) = (&ours, view.read(path)) {
        contents.ours.insert(path.to_string(), text);
    }
    let (options, note) = match &collision {
        Some(c) => (
            vec![Decision::Overwrite, Decision::Keep],
            Some(collision_note(c)),
        ),
        None if local == LocalState::Intact => (
            vec![Decision::Overwrite],
            Some("already identical on disk".to_string()),
        ),
        None => (vec![Decision::Overwrite], None),
    };
    PlanFile {
        path: path.to_string(),
        source: source.to_string(),
        kind: kind_of(path),
        id: id_of(kit, path, source),
        change: Change::Added,
        local,
        base: None,
        theirs: Some(theirs.to_string()),
        ours,
        collision,
        merge: None,
        conflicts: None,
        default: Some(Decision::Overwrite),
        options,
        skipped: false,
        note,
        agent: agent_entry(detail, source),
        agent_changes: None,
        body_changed: None,
        skill: skill_entry(detail, source),
    }
}

/// Workers the kit declares, against what Compose has.
fn plan_workers(
    view: &LocalView<'_>,
    kit: &str,
    detail: &KitDetail,
    previous: Option<&BTreeMap<String, String>>,
) -> Vec<PlanWorker> {
    let others = |name: &str| -> Vec<String> {
        view.lock
            .kits
            .iter()
            .filter(|(k, locked)| k.as_str() != kit && locked.workers.contains_key(name))
            .map(|(k, _)| k.clone())
            .collect()
    };
    let mut out = Vec::new();
    for w in &detail.workers {
        let declared = view.declared.get(&w.name);
        let range_from = previous.and_then(|p| p.get(&w.name)).cloned();
        let installed = declared.and_then(|d| d.resolved.clone());
        let action = match declared {
            None => WorkerAction::Add,
            Some(d) if d.path => WorkerAction::None,
            Some(d) => {
                let fits = installed.as_deref().and_then(|v| satisfies(&w.range, v));
                if fits == Some(false) {
                    WorkerAction::Update
                } else if range_from.as_deref().is_some_and(|from| from != w.range)
                    && d.selector.as_deref() == range_from.as_deref()
                {
                    WorkerAction::Redeclare
                } else {
                    WorkerAction::None
                }
            }
        };
        out.push(PlanWorker {
            name: w.name.clone(),
            range: Some(w.range.clone()),
            range_from: range_from.filter(|from| from != &w.range),
            installed,
            declared: declared.and_then(|d| d.selector.clone()),
            container: declared.map(|d| d.container.clone()),
            action,
            to: w.resolved.clone(),
            kind: w.kind.clone(),
            description: w.description.clone(),
            path: declared.is_some_and(|d| d.path),
            used_by: others(&w.name),
            registry_url: view.env.worker_web_url(&w.name),
        });
    }
    if let Some(previous) = previous {
        for (name, range) in previous {
            if detail.workers.iter().any(|w| &w.name == name) {
                continue;
            }
            let declared = view.declared.get(name);
            out.push(PlanWorker {
                name: name.clone(),
                range: None,
                range_from: Some(range.clone()),
                installed: declared.and_then(|d| d.resolved.clone()),
                declared: declared.and_then(|d| d.selector.clone()),
                container: declared.map(|d| d.container.clone()),
                action: if declared.is_some_and(|d| !d.path) {
                    WorkerAction::Remove
                } else {
                    WorkerAction::None
                },
                to: None,
                kind: None,
                description: None,
                path: declared.is_some_and(|d| d.path),
                used_by: others(name),
                registry_url: view.env.worker_web_url(name),
            });
        }
    }
    out
}

fn needs_compose(workers: &[PlanWorker]) -> bool {
    workers.iter().any(|w| {
        matches!(
            w.action,
            WorkerAction::Add | WorkerAction::Update | WorkerAction::Redeclare
        )
    })
}

fn compose_blocking() -> Issue {
    Issue {
        code: "compose_not_running".into(),
        path: None,
        message: "This kit adds or moves workers, which needs the Compose daemon. Start the \
                  project with `iii compose --up`, then review the plan again."
            .into(),
    }
}

fn deprecation_blocking(kit: &str, d: &Deprecation) -> Issue {
    let mut message = format!("{kit} is deprecated");
    if let Some(m) = d.message.as_deref().filter(|m| !m.is_empty()) {
        message.push_str(&format!(": {m}"));
    }
    if let Some(r) = &d.replaced_by {
        message.push_str(&format!(". Install {r} instead"));
    }
    message.push('.');
    Issue {
        code: "kit_deprecated".into(),
        path: None,
        message,
    }
}

fn agent_changes(old: Option<&AgentEntry>, new: Option<&AgentEntry>) -> AgentChanges {
    let mut out = AgentChanges::default();
    let (Some(old), Some(new)) = (old, new) else {
        return out;
    };
    let mut scalar = |field: &str, a: &Option<String>, b: &Option<String>| {
        if a != b {
            out.fields.insert(field.to_string(), (a.clone(), b.clone()));
        }
    };
    scalar("name", &old.name, &new.name);
    scalar("description", &old.description, &new.description);
    scalar("logo", &old.logo, &new.logo);
    scalar("model", &old.model, &new.model);
    scalar(
        "reasoning_effort",
        &old.reasoning_effort,
        &new.reasoning_effort,
    );
    scalar("extends", &old.extends, &new.extends);
    scalar("icon", &old.icon, &new.icon);
    scalar("color", &old.color, &new.color);
    let diff = |a: &[String], b: &[String]| -> (Vec<String>, Vec<String>) {
        let a: BTreeSet<&String> = a.iter().collect();
        let b: BTreeSet<&String> = b.iter().collect();
        (
            b.difference(&a).map(|s| (*s).clone()).collect(),
            a.difference(&b).map(|s| (*s).clone()).collect(),
        )
    };
    (out.functions_added, out.functions_removed) = diff(&old.functions, &new.functions);
    (out.skills_added, out.skills_removed) = diff(&old.skills, &new.skills);
    out
}

/// What the kit's profiles gain: new workers, newly preloaded functions,
/// model changes. `old` is `None` on install.
fn capabilities(old: Option<&KitDetail>, new: &KitDetail, workers: &[PlanWorker]) -> Capabilities {
    let workers_added = workers
        .iter()
        .filter(|w| w.action == WorkerAction::Add)
        .map(|w| w.name.clone())
        .collect();
    let mut functions_added = Vec::new();
    let mut models_changed = Vec::new();
    for agent in &new.agents {
        let before = old.and_then(|o| o.agents.iter().find(|a| a.id == agent.id));
        let known: BTreeSet<&String> = before
            .map(|b| b.functions.iter().collect())
            .unwrap_or_default();
        for f in &agent.functions {
            if !known.contains(f) {
                functions_added.push(FunctionAdded {
                    agent: agent.id.clone(),
                    function: f.clone(),
                });
            }
        }
        match before {
            Some(b) if b.model != agent.model => models_changed.push(ModelChanged {
                agent: agent.id.clone(),
                from: b.model.clone(),
                to: agent.model.clone(),
            }),
            None if old.is_none() && agent.model.is_some() => models_changed.push(ModelChanged {
                agent: agent.id.clone(),
                from: None,
                to: agent.model.clone(),
            }),
            _ => {}
        }
    }
    Capabilities {
        workers_added,
        functions_added,
        models_changed,
    }
}

fn finish(mut plan: Plan) -> Plan {
    let mut counts = PlanCounts {
        agents: plan
            .files
            .iter()
            .filter(|f| f.kind == FileKind::Agent && f.change != Change::Removed)
            .count(),
        skills: plan
            .files
            .iter()
            .filter(|f| f.kind == FileKind::Skill && f.change != Change::Removed)
            .count(),
        workers: plan.workers.iter().filter(|w| w.range.is_some()).count(),
        functions: plan.functions.len(),
        ..Default::default()
    };
    for f in &plan.files {
        match f.change {
            Change::Added => counts.added += 1,
            Change::Modified | Change::Kept => counts.modified += 1,
            Change::Removed => counts.removed += 1,
        }
        if f.collision.is_some() {
            counts.collisions += 1;
        }
        if f.merge == Some(MergeStatus::Conflicts) {
            counts.conflicts += 1;
        }
        if f.default.is_none() {
            counts.decisions_required += 1;
        }
    }
    counts.unchanged = plan.counts.unchanged;
    if plan.kind == PlanKind::Update {
        // Update counts describe the target version, not only what moves.
        counts.agents = plan.counts.agents;
        counts.skills = plan.counts.skills;
    }
    plan.counts = counts;
    plan.review = REVIEW_HINT.replace("<plan_id>", &plan.plan_id);
    plan
}

/// Build an install plan for `detail` (the resolved target version).
pub async fn build_install_plan(
    view: &LocalView<'_>,
    registry: &dyn KitRegistry,
    detail: &KitDetail,
    requested: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(Plan, PlanContents), String> {
    let kit = detail.id.clone();
    let files = registry.files(&kit, &detail.version).await?;
    let functions = registry
        .functions(&kit, &detail.version)
        .await
        .unwrap_or_default();
    let mut contents = PlanContents::default();
    let mut plan_files = Vec::new();
    let mut warnings = Vec::new();
    for (path, (source, sha, content)) in index_files(&kit, &files) {
        contents.blobs.insert(sha.clone(), content);
        let file = added_file(view, &kit, detail, &path, &source, &sha, &mut contents);
        if let Some(c) = &file.collision {
            warnings.push(collision_issue(&path, c));
        }
        plan_files.push(file);
    }
    sort_files(&mut plan_files);
    let workers = plan_workers(view, &kit, detail, None);
    let mut blocking = Vec::new();
    if let Some(d) = &detail.deprecation {
        blocking.push(deprecation_blocking(&kit, d));
    }
    if needs_compose(&workers) && !view.compose_available {
        blocking.push(compose_blocking());
    }
    let caps = capabilities(None, detail, &workers);
    let plan = Plan {
        plan_id: new_plan_id(),
        kind: PlanKind::Install,
        kit: kit.clone(),
        from: None,
        to: Some(detail.version.clone()),
        requested: requested.to_string(),
        bump: None,
        major: false,
        author: Some(detail.author.clone()),
        description: detail.description.clone(),
        license: detail.license.clone(),
        repo: detail.repo.clone(),
        published_at: detail.published_at.clone(),
        registry_url: view.env.kit_web_url(&kit),
        notes: detail.notes.clone(),
        deprecation: detail.deprecation.clone(),
        workers,
        files: plan_files,
        capabilities: caps,
        functions,
        warnings,
        blocking,
        counts: PlanCounts::default(),
        created_at: now_rfc3339(now),
        expires_at: now_rfc3339(now + chrono::Duration::hours(PLAN_TTL_HOURS)),
        review: String::new(),
    };
    Ok((finish(plan), contents))
}

/// Build an update plan from the installed `locked` version to `detail`.
pub async fn build_update_plan(
    view: &LocalView<'_>,
    registry: &dyn KitRegistry,
    locked: &LockedKit,
    detail: &KitDetail,
    requested: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(Plan, PlanContents), String> {
    let kit = detail.id.clone();
    let files = registry.files(&kit, &detail.version).await?;
    let functions = registry
        .functions(&kit, &detail.version)
        .await
        .unwrap_or_default();
    let old_detail = registry.detail(&kit, &locked.version).await.ok().flatten();
    let new_files = index_files(&kit, &files);
    let mut contents = PlanContents::default();
    let mut plan_files = Vec::new();
    let mut warnings = Vec::new();
    let mut unchanged = 0usize;

    let paths: BTreeSet<String> = new_files
        .keys()
        .cloned()
        .chain(locked.files.keys().cloned())
        .collect();
    for path in paths {
        let new = new_files.get(&path);
        let old = locked.files.get(&path);
        let abs = view.env.abs(&path);
        let local = abs.as_deref().and_then(local_sha);
        match (old, new) {
            (None, Some((source, sha, content))) => {
                contents.blobs.insert(sha.clone(), content.clone());
                let file = added_file(view, &kit, detail, &path, source, sha, &mut contents);
                if let Some(c) = &file.collision {
                    warnings.push(collision_issue(&path, c));
                }
                plan_files.push(file);
            }
            (Some(old), Some((source, sha, content))) => {
                if &old.sha256 == sha {
                    unchanged += 1;
                    continue;
                }
                contents.blobs.insert(sha.clone(), content.clone());
                let agent = agent_entry(detail, source);
                let old_agent = old_detail.as_ref().and_then(|d| agent_entry(d, source));
                let changes = (agent.is_some() || old_agent.is_some())
                    .then(|| agent_changes(old_agent.as_ref(), agent.as_ref()));
                let mut file = PlanFile {
                    path: path.clone(),
                    source: source.clone(),
                    kind: kind_of(&path),
                    id: id_of(&kit, &path, source),
                    change: Change::Modified,
                    local: LocalState::Intact,
                    base: Some(old.sha256.clone()),
                    theirs: Some(sha.clone()),
                    ours: local.clone(),
                    collision: None,
                    merge: None,
                    conflicts: None,
                    default: Some(Decision::Kit),
                    options: vec![Decision::Kit],
                    skipped: old.skipped,
                    note: None,
                    agent,
                    agent_changes: changes,
                    body_changed: None,
                    skill: skill_entry(detail, source),
                };
                if old.skipped {
                    file.change = Change::Kept;
                    file.local = if local.is_some() {
                        LocalState::Occupied
                    } else {
                        LocalState::Absent
                    };
                    file.default = Some(Decision::Keep);
                    file.options = vec![Decision::Keep, Decision::Overwrite];
                    file.note = Some(
                        "you kept the existing file at install; the kit's version changed".into(),
                    );
                    if let Some(text) = view.read(&path) {
                        contents.ours.insert(path.clone(), text);
                    }
                    plan_files.push(file);
                    continue;
                }
                let base_text = registry.blob(&old.sha256).await.ok().flatten();
                if let Some(base) = &base_text {
                    contents.blobs.insert(old.sha256.clone(), base.clone());
                    if file.kind == FileKind::Agent {
                        file.body_changed = Some(
                            crate::fs_source::split_frontmatter(base).1
                                != crate::fs_source::split_frontmatter(content).1,
                        );
                    }
                }
                match file_state_of(old, local.as_deref()) {
                    FileState::Missing => {
                        file.local = LocalState::Missing;
                        file.options = vec![Decision::Kit, Decision::Mine];
                        file.note = Some(
                            "you deleted this file; the update restores it unless you keep it deleted"
                                .into(),
                        );
                    }
                    FileState::Edited => {
                        file.local = LocalState::Edited;
                        let ours_text = view.read(&path).unwrap_or_default();
                        contents.ours.insert(path.clone(), ours_text.clone());
                        file.options = vec![Decision::Kit, Decision::Mine, Decision::Merged];
                        match &base_text {
                            Some(base) => {
                                let merged = three_way(base, &ours_text, content);
                                file.merge = Some(merged.status);
                                if merged.status == MergeStatus::Clean {
                                    file.default = Some(Decision::Merged);
                                    file.note = Some(
                                        "edited by you and changed in the kit; the changes merge cleanly"
                                            .into(),
                                    );
                                    warnings.push(Issue {
                                        code: "local_edits_merged".into(),
                                        path: Some(path.clone()),
                                        message: format!(
                                            "{path} has local edits; the update merges them with the kit's changes."
                                        ),
                                    });
                                } else {
                                    file.default = None;
                                    file.conflicts = Some(merged.conflicts);
                                    file.note = Some(
                                        "edited by you and changed in the kit; the changes conflict"
                                            .into(),
                                    );
                                    warnings.push(Issue {
                                        code: "merge_conflict".into(),
                                        path: Some(path.clone()),
                                        message: format!(
                                            "{path} was edited here and changed in the kit; {} conflict{} need{} a decision.",
                                            merged.conflicts,
                                            if merged.conflicts == 1 { "" } else { "s" },
                                            if merged.conflicts == 1 { "s" } else { "" }
                                        ),
                                    });
                                }
                                contents.merged.insert(path.clone(), merged.content);
                            }
                            None => {
                                file.merge = Some(MergeStatus::Conflicts);
                                file.default = None;
                                file.note = Some(
                                    "edited by you; the installed version is no longer in the registry, so it cannot be merged"
                                        .into(),
                                );
                                warnings.push(Issue {
                                    code: "merge_conflict".into(),
                                    path: Some(path.clone()),
                                    message: format!(
                                        "{path} was edited here and its base is unavailable; choose the kit's version or yours."
                                    ),
                                });
                                file.options = vec![Decision::Kit, Decision::Mine];
                            }
                        }
                    }
                    _ => {}
                }
                plan_files.push(file);
            }
            (Some(old), None) => {
                if old.skipped {
                    continue;
                }
                let source = source_for_install_path(&kit, &path).unwrap_or_else(|| path.clone());
                let state = file_state_of(old, local.as_deref());
                if state == FileState::Missing {
                    // Already gone: the lock simply forgets it.
                    continue;
                }
                if let Some(base) = registry.blob(&old.sha256).await.ok().flatten() {
                    contents.blobs.insert(old.sha256.clone(), base);
                }
                let edited = state == FileState::Edited;
                if edited {
                    if let Some(text) = view.read(&path) {
                        contents.ours.insert(path.clone(), text);
                    }
                }
                plan_files.push(PlanFile {
                    path: path.clone(),
                    source: source.clone(),
                    kind: kind_of(&path),
                    id: id_of(&kit, &path, &source),
                    change: Change::Removed,
                    local: if edited {
                        LocalState::Edited
                    } else {
                        LocalState::Intact
                    },
                    base: Some(old.sha256.clone()),
                    theirs: None,
                    ours: local.clone(),
                    collision: None,
                    merge: None,
                    conflicts: None,
                    default: Some(if edited {
                        Decision::Keep
                    } else {
                        Decision::Remove
                    }),
                    options: vec![Decision::Remove, Decision::Keep],
                    skipped: false,
                    note: Some(if edited {
                        "removed from the kit; you edited it, so it stays as a local file unless you remove it".into()
                    } else {
                        "removed from the kit".into()
                    }),
                    agent: old_detail.as_ref().and_then(|d| agent_entry(d, &source)),
                    agent_changes: None,
                    body_changed: None,
                    skill: old_detail.as_ref().and_then(|d| skill_entry(d, &source)),
                });
            }
            (None, None) => {}
        }
    }
    sort_files(&mut plan_files);

    let workers = plan_workers(view, &kit, detail, Some(&locked.workers));
    let mut blocking = Vec::new();
    if let Some(d) = &detail.deprecation {
        blocking.push(deprecation_blocking(&kit, d));
    }
    if needs_compose(&workers) && !view.compose_available {
        blocking.push(compose_blocking());
    }
    let major = is_major_jump(&locked.version, &detail.version);
    if major {
        warnings.insert(
            0,
            Issue {
                code: "major_update".into(),
                path: None,
                message: format!(
                    "{} → {} is a major version: expect breaking changes.",
                    locked.version, detail.version
                ),
            },
        );
    }
    let caps = capabilities(old_detail.as_ref(), detail, &workers);
    let plan = Plan {
        plan_id: new_plan_id(),
        kind: PlanKind::Update,
        kit: kit.clone(),
        from: Some(locked.version.clone()),
        to: Some(detail.version.clone()),
        requested: requested.to_string(),
        bump: bump_kind(&locked.version, &detail.version).map(str::to_string),
        major,
        author: Some(detail.author.clone()),
        description: detail.description.clone(),
        license: detail.license.clone(),
        repo: detail.repo.clone(),
        published_at: detail.published_at.clone(),
        registry_url: view.env.kit_web_url(&kit),
        notes: detail.notes.clone(),
        deprecation: detail.deprecation.clone(),
        workers,
        files: plan_files,
        capabilities: caps,
        functions,
        warnings,
        blocking,
        counts: PlanCounts {
            unchanged,
            agents: new_files
                .keys()
                .filter(|p| p.starts_with("agents/"))
                .count(),
            skills: new_files
                .keys()
                .filter(|p| p.starts_with("skills/"))
                .count(),
            ..Default::default()
        },
        created_at: now_rfc3339(now),
        expires_at: now_rfc3339(now + chrono::Duration::hours(PLAN_TTL_HOURS)),
        review: String::new(),
    };
    Ok((finish(plan), contents))
}

/// Build a removal plan for an installed kit. Edited files are kept by
/// default; workers are listed but never removed unless asked.
pub fn build_remove_plan(
    view: &LocalView<'_>,
    kit: &str,
    locked: &LockedKit,
    now: chrono::DateTime<chrono::Utc>,
) -> (Plan, PlanContents) {
    let mut contents = PlanContents::default();
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    for (path, entry) in &locked.files {
        if entry.skipped {
            continue;
        }
        let local = view.env.abs(path).as_deref().and_then(local_sha);
        let state = file_state_of(entry, local.as_deref());
        if state == FileState::Missing {
            continue;
        }
        let edited = state == FileState::Edited;
        let source = source_for_install_path(kit, path).unwrap_or_else(|| path.clone());
        if let Some(text) = view.read(path) {
            contents.ours.insert(path.clone(), text);
        }
        if edited {
            warnings.push(Issue {
                code: "edited_file_kept".into(),
                path: Some(path.clone()),
                message: format!(
                    "{path} was edited in this project; it is kept unless you choose to delete it."
                ),
            });
        }
        files.push(PlanFile {
            path: path.clone(),
            source: source.clone(),
            kind: kind_of(path),
            id: id_of(kit, path, &source),
            change: Change::Removed,
            local: if edited {
                LocalState::Edited
            } else {
                LocalState::Intact
            },
            base: Some(entry.sha256.clone()),
            theirs: None,
            ours: local,
            collision: None,
            merge: None,
            conflicts: None,
            default: Some(if edited {
                Decision::Keep
            } else {
                Decision::Remove
            }),
            options: vec![Decision::Remove, Decision::Keep],
            skipped: false,
            note: edited.then(|| "edited by you".to_string()),
            agent: None,
            agent_changes: None,
            body_changed: None,
            skill: None,
        });
    }
    sort_files(&mut files);
    let others = |name: &str| -> Vec<String> {
        view.lock
            .kits
            .iter()
            .filter(|(k, l)| k.as_str() != kit && l.workers.contains_key(name))
            .map(|(k, _)| k.clone())
            .collect()
    };
    let workers = locked
        .workers
        .iter()
        .map(|(name, range)| {
            let declared = view.declared.get(name);
            PlanWorker {
                name: name.clone(),
                range: None,
                range_from: Some(range.clone()),
                installed: declared.and_then(|d| d.resolved.clone()),
                declared: declared.and_then(|d| d.selector.clone()),
                container: declared.map(|d| d.container.clone()),
                action: if declared.is_some_and(|d| !d.path) {
                    WorkerAction::Remove
                } else {
                    WorkerAction::None
                },
                to: None,
                kind: None,
                description: None,
                path: declared.is_some_and(|d| d.path),
                used_by: others(name),
                registry_url: view.env.worker_web_url(name),
            }
        })
        .collect();
    let plan = Plan {
        plan_id: new_plan_id(),
        kind: PlanKind::Remove,
        kit: kit.to_string(),
        from: Some(locked.version.clone()),
        to: None,
        requested: locked.requested.clone(),
        bump: None,
        major: false,
        author: None,
        description: None,
        license: None,
        repo: None,
        published_at: None,
        registry_url: view.env.kit_web_url(kit),
        notes: None,
        deprecation: None,
        workers,
        files,
        capabilities: Capabilities::default(),
        functions: Vec::new(),
        warnings,
        blocking: Vec::new(),
        counts: PlanCounts::default(),
        created_at: now_rfc3339(now),
        expires_at: now_rfc3339(now + chrono::Duration::hours(PLAN_TTL_HOURS)),
        review: String::new(),
    };
    (finish(plan), contents)
}

fn file_state_of(entry: &super::lock::LockedFile, local: Option<&str>) -> FileState {
    let unskipped = super::lock::LockedFile {
        sha256: entry.sha256.clone(),
        skipped: false,
    };
    super::lock::file_state(&unskipped, local)
}

/// Review order: profiles before skills, then by path.
fn sort_files(files: &mut [PlanFile]) {
    files.sort_by(|a, b| {
        let rank = |f: &PlanFile| (f.kind == FileKind::Skill) as u8;
        rank(a).cmp(&rank(b)).then_with(|| a.path.cmp(&b.path))
    });
}

/// Compact form for lists and chat cards: no file entries.
pub fn summary(plan: &Plan) -> Value {
    json!({
        "plan_id": plan.plan_id,
        "kind": plan.kind,
        "kit": plan.kit,
        "from": plan.from,
        "to": plan.to,
        "major": plan.major,
        "counts": plan.counts,
        "warnings": plan.warnings.len(),
        "blocking": plan.blocking.len(),
        "created_at": plan.created_at,
        "expires_at": plan.expires_at,
    })
}

/// Can `download-kit apply=true` apply this plan without a human?
pub fn auto_applicable(plan: &Plan) -> bool {
    plan.warnings.is_empty() && plan.blocking.is_empty() && plan.counts.decisions_required == 0
}
