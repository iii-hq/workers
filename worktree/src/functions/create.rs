//! `worktree::create` — mint an isolated, locked worktree and register it.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{codes, WError};
use crate::events::{CreatedEvent, EventCtx, EventKind};
use crate::functions::Deps;
use crate::git::{canonical_or_self, ops, validate_abs_path, validate_ref_name};
use crate::ids;
use crate::state;
use crate::types::{now_ms, Lifecycle, WorktreeRecord};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// Absolute path of the repository to branch from.
    pub repo_path: String,
    /// Ref to base the worktree on. Defaults to `HEAD`.
    #[serde(default)]
    pub base_ref: Option<String>,
    /// Branch to create. Defaults to `<branch_prefix><worktree_id>` (or a
    /// codename when `branch_naming: codename`).
    #[serde(default)]
    pub branch: Option<String>,
    /// Create the worktree at a GitHub pull request head: fetches
    /// `refs/pull/<n>/head` from `origin` and branches `<prefix>pr-<n>` at
    /// it. Mutually exclusive with `base_ref` and `branch`.
    #[serde(default)]
    pub pr: Option<u64>,
    /// Session to auto-claim the worktree for.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// Stable id, `wt_<12hex>`.
    pub worktree_id: String,
    /// Absolute path of the new worktree; hand this to an agent as its working directory.
    pub path: String,
    /// The branch checked out in the worktree.
    pub branch: String,
    /// The ref the worktree was based on.
    pub base_ref: String,
    /// The commit `base_ref` resolved to.
    pub base_sha: String,
    /// Canonical per-repo key (FIFO group for lands).
    pub repo_key: String,
    /// Advisory dev-server port derived from the worktree id.
    pub dev_port: u16,
    /// Creation time, ms since epoch.
    pub created_at: i64,
}

pub async fn handle(deps: &Deps, req: Request) -> Result<Response, WError> {
    if req.pr.is_some() && (req.base_ref.is_some() || req.branch.is_some()) {
        return Err(WError::new(
            codes::INVALID_REQUEST,
            "pr is mutually exclusive with base_ref and branch",
        ));
    }

    let cfg = deps.cfg().await;
    let t = cfg.git_timeout_ms;

    let repo = validate_abs_path("repo_path", &req.repo_path)?;
    if !repo.is_dir() {
        return Err(WError::new(
            codes::BAD_PATH,
            format!("repo_path is not a directory: {}", repo.display()),
        ));
    }
    // Gate on the canonicalized path so symlink spellings cannot dodge the
    // allowlist. Checked at create only: narrowing the list later never
    // breaks existing worktrees.
    let canonical_repo = canonical_or_self(&repo);
    if !cfg.gates.repo_allowed(&canonical_repo.to_string_lossy()) {
        return Err(crate::functions::allowlist_denied(
            codes::REPO_NOT_ALLOWED,
            &format!("repository {}", canonical_repo.display()),
            "gates.repos",
        ));
    }

    let repo_key = ops::git_common_dir(&repo, t).await?;
    let _guard = deps.locks.guard(&repo_key).await;

    // Budget check under the repo lock so concurrent creates serialize
    // against the same count. Orphaned records are not live worktrees.
    if cfg.gates.max_worktrees_per_repo > 0 {
        let live = state::collect_records(deps.state.as_ref())
            .await?
            .iter()
            .filter(|r| r.repo_key == repo_key && r.lifecycle != Lifecycle::Orphaned)
            .count() as u32;
        if live >= cfg.gates.max_worktrees_per_repo {
            return Err(crate::functions::budget_denied(&canonical_repo, live));
        }
    }

    let worktree_id = ids::mint_worktree_id();
    let (base_ref, base_sha, branch, branch_taken) = match req.pr {
        Some(n) => {
            let branch = format!("{}pr-{n}", cfg.branch_prefix);
            validate_ref_name("branch", &branch)?;
            let sha = ops::fetch_pr_head(&repo, n, t).await?;
            let taken = ops::branch_exists(&repo, &branch, t).await?;
            (format!("refs/pull/{n}/head"), sha, branch, taken)
        }
        None => {
            let base_ref = req.base_ref.unwrap_or_else(|| "HEAD".to_string());
            validate_ref_name("base_ref", &base_ref)?;
            let branch = req
                .branch
                .unwrap_or_else(|| cfg.default_branch(&worktree_id));
            validate_ref_name("branch", &branch)?;
            // Independent read-only lookups; run them concurrently.
            let (sha, taken) = tokio::join!(
                ops::rev_parse(&repo, &base_ref, t),
                ops::branch_exists(&repo, &branch, t),
            );
            (base_ref, sha?, branch, taken?)
        }
    };
    if branch_taken {
        return Err(WError::new(
            codes::BRANCH_EXISTS,
            format!("branch {branch:?} already exists"),
        ));
    }

    let root = cfg.expanded_worktree_root();
    let dir = ids::worktree_dir(&root, &repo_key, &worktree_id);
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            WError::new(
                codes::BAD_PATH,
                format!("create worktree parent {}: {e}", parent.display()),
            )
        })?;
    }

    ops::worktree_add(&repo, &dir, &branch, &base_sha, &worktree_id, t).await?;
    // Store the canonical path so comparisons against git's own worktree
    // listing match exactly.
    let dir = canonical_or_self(&dir);

    let now = now_ms();
    let record = WorktreeRecord {
        worktree_id: worktree_id.clone(),
        repo_path: repo.to_string_lossy().into_owned(),
        repo_key: repo_key.clone(),
        path: dir.to_string_lossy().into_owned(),
        branch: branch.clone(),
        base_ref: base_ref.clone(),
        base_sha: base_sha.clone(),
        lifecycle: if req.session_id.is_some() {
            Lifecycle::Claimed
        } else {
            Lifecycle::Active
        },
        session_id: req.session_id.clone(),
        created_at: now,
        updated_at: now,
    };
    if let Err(e) = state::put_record(deps.state.as_ref(), &record).await {
        // The registry write failed; undo the git side so no unmanaged
        // worktree is left behind.
        ops::worktree_unlock(&repo, &dir, t).await;
        if let Err(cleanup) = ops::worktree_remove(&repo, &dir, true, t).await {
            tracing::warn!(error = %cleanup, "cleanup after failed record write also failed");
        }
        ops::branch_delete(&repo, &branch, true, t).await;
        return Err(e);
    }

    deps.emitter
        .emit(
            EventKind::Created,
            EventCtx {
                repo_path: &record.repo_path,
                worktree_id: &worktree_id,
                session_id: record.session_id.as_deref(),
            },
            &CreatedEvent {
                worktree_id: worktree_id.clone(),
                repo_path: record.repo_path.clone(),
                path: record.path.clone(),
                branch: branch.clone(),
                base_ref: base_ref.clone(),
                base_sha: base_sha.clone(),
                session_id: record.session_id.clone(),
                timestamp: now,
            },
        )
        .await;

    if cfg.provision.copy_ignored {
        // Best-effort background provisioning; the create response never
        // waits on it and nothing new is emitted.
        crate::provision::spawn_copy_ignored(
            repo.clone(),
            std::path::PathBuf::from(&record.path),
            cfg.provision.clone(),
            root,
            t,
        );
    }

    Ok(Response {
        dev_port: ids::dev_port(&worktree_id),
        worktree_id,
        path: record.path,
        branch,
        base_ref,
        base_sha,
        repo_key,
        created_at: now,
    })
}

/// Path guard shared with the other handlers: the record must exist.
pub async fn require_record(
    deps: &Deps,
    worktree_id: &str,
) -> Result<crate::types::WorktreeRecord, WError> {
    state::get_record(deps.state.as_ref(), worktree_id)
        .await?
        .ok_or_else(|| {
            WError::new(
                codes::NOT_FOUND,
                format!("no worktree record for {worktree_id:?}"),
            )
        })
}

/// The record's directory must still exist.
pub fn require_dir(record: &crate::types::WorktreeRecord) -> Result<(), WError> {
    if !Path::new(&record.path).is_dir() {
        return Err(WError::new(
            codes::PATH_MISSING,
            format!(
                "worktree {} directory is missing ({}); run worktree::prune to reconcile",
                record.worktree_id, record.path
            ),
        ));
    }
    Ok(())
}
