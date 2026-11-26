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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
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

impl State {
    pub fn find_check(&self, id: &str) -> Option<&Check> {
        self.check
            .iter()
            .chain(self.checks.iter())
            .find(|c| c.id == id)
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
        let referenced = |s: &State, c: &Check| {
            s.merges.iter().any(|m| m.check_id == c.id)
                || s.handoffs
                    .iter()
                    .any(|h| h.check_id.as_deref() == Some(c.id.as_str()))
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
        if latest {
            self.check = Some(check);
        }
    }

    /// Put a used project first; one entry per folder and cloud project.
    pub fn remember_project(&mut self, recent: RecentProject) {
        self.recent_projects
            .retain(|r| !(r.path == recent.path && r.cloud_project_id == recent.cloud_project_id));
        self.recent_projects.insert(0, recent);
        self.recent_projects.truncate(MAX_RECENT_PROJECTS);
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
