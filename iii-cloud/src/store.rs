//! The worker's records, in one JSON file (`<data_dir>/state.json`) written to
//! a temp file and renamed (spec D4). The file holds no secret value. Missing
//! fields default, so a hand-written fixture needs only what it shows.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::WorkerError;
use crate::types::*;

/// A pull request this worker merged and has not turned into a deployment yet.
/// Saved right after the merge so a second `deploy` resumes from the pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingMerge {
    pub check_id: String,
    pub pr: u64,
    pub target: String,
    /// When GitHub says the merge happened: deployments before it are not the merge's.
    #[serde(default)]
    pub merged_at: Option<String>,
}

/// One `override::choose`, for the values it was made for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredChoice {
    pub console_hash: String,
    pub file_hash: String,
    pub choice: OverrideChoice,
}

/// What the workspace was on when a check was stored: the source branch or ref
/// and the environment's compose file. A deploy of the check is refused when
/// they are not the same any more (`SOURCE_CHANGED`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckedFor {
    pub source: Option<String>,
    /// The commit the source pointed at when the workspace took it (a branch
    /// switched in the person's checkout changes it with no name given).
    #[serde(default)]
    pub source_sha: Option<String>,
    /// The source above was read from the selected workspace. A check made while
    /// another environment was selected has none: the source half does not apply.
    #[serde(default)]
    pub workspace: bool,
    pub compose: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// `CheckedFor` by check id (kept with the checks). A check with none (made
    /// before this was recorded) is never refused for it.
    pub check_sources: std::collections::BTreeMap<String, CheckedFor>,
    pub merges: Vec<PendingMerge>,
    /// Ids of the deployments this worker made or adopted from its own merge.
    pub worker_deploys: Vec<String>,
    /// Hand-offs to the agent, one per environment.
    pub handoffs: Vec<Handoff>,
    /// When each fresh worktree was made (key: `<path>\n<branch>`): pull
    /// requests created before are an earlier try's, not this one's.
    pub worktree_started: std::collections::BTreeMap<String, String>,
    /// The base branch and sha when each fresh worktree was made (same key).
    /// Failed creates per idempotency key: a retry after a failure is a new key.
    pub create_attempts: std::collections::BTreeMap<String, u32>,
    pub worktree_base: std::collections::BTreeMap<String, (String, String)>,
    /// `Workspace.from_head` per worktree path: kept while the worktree is reused.
    pub worktree_from_head: std::collections::BTreeMap<String, String>,
    /// `Workspace.source` per worktree path, kept with `worktree_from_head`.
    pub worktree_source: std::collections::BTreeMap<String, String>,
    /// Where each kept deploy was made from, by deploy id.
    pub deploy_origins: std::collections::BTreeMap<String, DeployOrigin>,
    /// The names the person skipped for now (`values::skip`), per environment
    /// id. Names only, never a value.
    pub skipped_values: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    /// The warning codes the person skipped for now (`problem::skip`), per
    /// environment id.
    pub skipped_problems: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    /// The person's override choices (`override::choose`), per environment id
    /// and setting, with the hashes of the console and file values they were
    /// made for: a changed pair no longer counts.
    pub override_choices:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, StoredChoice>>,
    /// `Workspace.source_sha` per worktree path, kept with `worktree_from_head`.
    pub worktree_source_sha: std::collections::BTreeMap<String, String>,
    pub connection: Option<Connection>,
    /// The device sign-in being polled: its code and interval, so a restart can resume it.
    /// The cloud removed the selected project or environment (soft delete).
    pub project_deleted: bool,
    pub environment_deleted: bool,
    pub pending_device: Option<(String, u64)>,
    pub project: Option<Project>,
    /// A folder `project::resolve` read while another project is selected:
    /// Setup's add-project steps show it. `project::use` selects it and clears this.
    pub candidate: Option<Project>,
    /// The selected environment's latest check.
    pub check: Option<Check>,
    /// Earlier checks, newest first, at most 20 (a deploy points at one).
    pub checks: Vec<Check>,
    /// What each run was made from (a hash of the commit, compose file, engine
    /// and run environment), by run id: a passed run with the same key is reused.
    /// Only runs some kept check holds.
    pub run_keys: std::collections::BTreeMap<String, String>,
    pub secrets: Vec<SecretRequest>,
    pub variables: Vec<VariableRow>,
    pub pr: Option<PullRequest>,
    pub pr_error: Option<WorkerError>,
    pub deploys: Vec<Deploy>,
    pub refreshed_at: Option<String>,
    /// What the last refresh read: `iii --version` here, the cloud's engine.
    pub iii_version: Option<String>,
    pub cloud_engine: Option<String>,
    /// Pushes made elsewhere are followed when created after this instant.
    pub followed_since: Option<String>,
    /// The projects `project::use` selected, newest first, at most 8.
    pub recent_projects: Vec<RecentProject>,
}

const MAX_CHECKS: usize = 20;
const MAX_DEPLOYS: usize = 20;
const MAX_RECENT_PROJECTS: usize = 8;

/// Not stopped, and not a finished one that has been quiet for 30 minutes (retired).
fn handoff_going(h: &Handoff) -> bool {
    if h.agent.state == AgentPhase::Stopped {
        return false;
    }
    let quiet = h
        .agent
        .last_activity_at
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp_millis())
        .unwrap_or(h.asked_at_ms);
    h.agent.state != AgentPhase::Done
        || chrono::Utc::now().timestamp_millis() - quiet <= 30 * 60 * 1000
}

impl State {
    pub fn find_check(&self, id: &str) -> Option<&Check> {
        self.check
            .iter()
            .chain(self.checks.iter())
            .find(|c| c.id == id)
    }

    /// The compose file `env_id`'s work is on: its newest check's, else none
    /// (the caller falls back to the environment's own). One answer for every
    /// caller; naming a file in a request is how it changes. `current` is the
    /// file iii cloud has for the environment now: when it is not the one the
    /// environment had when that check was made, the file was changed in the
    /// console, and the check's file is not kept (it would undo that change).
    pub fn work_file(&self, env_id: &str, current: Option<&str>) -> Option<String> {
        let c = self.checks.iter().find(|c| c.environment_id == env_id)?;
        let at_check = self
            .check_sources
            .get(&c.id)
            .and_then(|r| r.compose.as_deref());
        if at_check.is_some() && current.is_some() && at_check != current {
            return None;
        }
        c.manifest.clone()
    }

    /// The environment's file is `file` now (this worker just switched it): every check
    /// of that environment was made for another, so a later change back in the
    /// console is seen as a change by `work_file`.
    pub fn env_file_is(&mut self, env_id: &str, file: &str) {
        let ids: Vec<String> = self
            .check_sources
            .keys()
            .filter(|id| {
                self.find_check(id)
                    .is_some_and(|c| c.environment_id == env_id)
            })
            .cloned()
            .collect();
        for id in ids {
            if let Some(r) = self.check_sources.get_mut(&id) {
                r.compose = Some(file.to_string());
            }
        }
    }

    /// Record `check` as the latest and keep it in the history.
    pub fn put_check(&mut self, check: Check) {
        self.put_check_for(check, true);
    }

    /// Keep a check in the history; it is also the environment's latest when `latest`.
    /// A check something still points at (a merge, a hand-off, an open PR, a
    /// deploy of its commit) is never dropped by the cap.
    pub fn put_check_for(&mut self, check: Check, latest: bool) {
        self.checks.retain(|c| c.id != check.id);
        self.checks.insert(0, check.clone());
        // Recorded once, when the check is first kept: a later update of its run is not a new source.
        if let Some(now) = self.checked_for_now(&check.environment_id) {
            self.check_sources.entry(check.id.clone()).or_insert(now);
        }
        let referenced = |s: &State, c: &Check| {
            s.merges.iter().any(|m| m.check_id == c.id)
                || s.handoffs
                    .iter()
                    .any(|h| h.check_id.as_deref() == Some(c.id.as_str()))
                // The checks of a hand-off that is still going: its agent's passed run is among them.
                || s.handoffs.iter().any(|h| {
                    c.handoff_id.as_deref() == Some(h.id.as_str()) && handoff_going(h)
                })
                || s.pr.as_ref().is_some_and(|p| p.head_sha == c.commit)
                || s.deploys.iter().any(|d| d.commit.sha == c.commit)
        };
        let mut kept = 0;
        let all = std::mem::take(&mut self.checks);
        self.checks = all
            .into_iter()
            .filter(|c| {
                kept += 1;
                kept <= MAX_CHECKS || referenced(self, c)
            })
            .collect();
        let kept: std::collections::BTreeSet<&str> =
            self.checks.iter().map(|c| c.id.as_str()).collect();
        self.check_sources
            .retain(|id, _| kept.contains(id.as_str()));
        if latest {
            self.check = Some(check);
        }
    }

    /// What the selected workspace is on now, when it is of environment `env_id`:
    /// `check` reads it when it starts, so a `use` that lands while it runs is not its source.
    pub fn checked_for_now(&self, env_id: &str) -> Option<CheckedFor> {
        let p = self.project.as_ref()?;
        let e = p.environment.as_ref().filter(|e| e.id == env_id)?;
        let w = p.workspace.as_ref();
        Some(CheckedFor {
            source: w.and_then(|w| w.source.clone()),
            source_sha: w.and_then(|w| w.source_sha.clone()),
            workspace: true,
            compose: e.manifest.clone(),
        })
    }

    /// Whether the workspace is on what `check_id` was made for. A check with
    /// no record (or of another environment than the selected one) passes.
    pub fn source_unchanged(&self, check_id: &str) -> bool {
        let (Some(rec), Some(p)) = (self.check_sources.get(check_id), self.project.as_ref()) else {
            return true;
        };
        let (Some(env), Some(check)) = (p.environment.as_ref(), self.find_check(check_id)) else {
            return true;
        };
        if env.id != check.environment_id {
            return true;
        }
        let now = self.checked_for_now(&env.id);
        let Some(now) = now else {
            return true;
        };
        // The compose file is compared in `deploy`, against the cloud's current one:
        // this cache is only as new as the last refresh.
        !rec.workspace || (now.source == rec.source && now.source_sha == rec.source_sha)
    }

    /// Put a used project first; one entry per folder and cloud project.
    pub fn remember_project(&mut self, recent: RecentProject) {
        self.recent_projects
            .retain(|r| !(r.path == recent.path && r.cloud_project_id == recent.cloud_project_id));
        self.recent_projects.insert(0, recent);
        self.recent_projects.truncate(MAX_RECENT_PROJECTS);
    }

    /// A deployment made in the console: a manual one this worker did not make
    /// or adopt (a teammate's push deployment is not).
    pub fn from_console(&self, deploy: &Deploy) -> bool {
        deploy.kind == "manual" && !self.worker_deploys.contains(&deploy.id)
    }

    pub fn put_deploy(&mut self, deploy: Deploy) {
        match self.deploys.iter_mut().find(|d| d.id == deploy.id) {
            Some(slot) => *slot = deploy,
            None => self.deploys.insert(0, deploy),
        }
        self.deploys.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        // The newest ones, and every one still moving, whatever its project.
        let mut kept = 0;
        self.deploys.retain(|d| {
            kept += 1;
            kept <= MAX_DEPLOYS || d.is_moving()
        });
        let ids: std::collections::BTreeSet<&str> =
            self.deploys.iter().map(|d| d.id.as_str()).collect();
        self.deploy_origins
            .retain(|id, _| ids.contains(id.as_str()));
    }

    pub fn environment_id(&self) -> Option<&str> {
        self.project
            .as_ref()
            .and_then(|p| p.environment.as_ref())
            .map(|e| e.id.as_str())
    }
}

pub struct Store {
    path: PathBuf,
    state: Mutex<State>,
}

impl Store {
    /// Open `<dir>/state.json`; a missing or unreadable file starts empty.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("state.json");
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                // Never replace it silently: it holds the merges not yet turned into deployments.
                let bad = path.with_extension("json.bad");
                tracing::error!(error = %e, file = %bad.display(), "state.json does not parse; kept as state.json.bad");
                let _ = std::fs::rename(&path, &bad);
                State::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => {
                // Unreadable: do not start over on top of it.
                let bad = path.with_extension("json.bad");
                tracing::error!(error = %e, file = %bad.display(), "state.json is unreadable; kept as state.json.bad");
                let _ = std::fs::rename(&path, &bad);
                State::default()
            }
        };
        Self {
            path,
            state: Mutex::new(state),
        }
    }

    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    pub fn read<T>(&self, f: impl FnOnce(&State) -> T) -> T {
        f(&self.state.lock().unwrap())
    }

    /// Change the state and persist it atomically.
    pub fn update<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        let mut state = self.state.lock().unwrap();
        let out = f(&mut state);
        if let Err(e) = self.persist(&state) {
            tracing::warn!(error = %e, "could not write state.json");
        }
        out
    }

    fn persist(&self, state: &State) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
        std::fs::rename(&tmp, &self.path)
    }
}
