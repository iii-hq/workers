//! `worktree::prune` — the reconcile sweep. Every request field defaults so
//! a cron binding can fire it with an empty `{}` payload.

use std::collections::BTreeSet;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::WError;
use crate::events::{EventCtx, EventKind, RemovedEvent};
use crate::functions::Deps;
use crate::git::{ops, validate_abs_path};
use crate::state;
use crate::types::{now_ms, Lifecycle};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct Request {
    /// Only sweep worktrees of this repository.
    #[serde(default)]
    pub repo_path: Option<String>,
    /// Report what would be pruned without acting.
    #[serde(default)]
    pub dry_run: bool,
}

/// One record the sweep left alone, and why.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PruneSkip {
    /// The skipped worktree.
    pub id: String,
    /// Why it was skipped.
    pub reason: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// Worktree ids pruned (or prunable, with dry_run).
    pub pruned: Vec<String>,
    /// Records left alone, with reasons.
    pub skipped: Vec<PruneSkip>,
}

pub async fn handle(deps: &Deps, req: Request) -> Result<Response, WError> {
    let cfg = deps.cfg().await;
    let t = cfg.git_timeout_ms;
    let expire_ms = (cfg.prune_expire_hours as i64).saturating_mul(3_600_000);

    let repo_key_filter = match &req.repo_path {
        Some(raw) => {
            let repo = validate_abs_path("repo_path", raw)?;
            Some(ops::git_common_dir(&repo, t).await?)
        }
        None => None,
    };

    let now = now_ms();
    let mut pruned = Vec::new();
    let mut skipped = Vec::new();
    let mut touched_repos: BTreeSet<String> = BTreeSet::new();

    for record in state::list_records(deps.state.as_ref()).await? {
        if let Some(repo_key) = &repo_key_filter {
            if &record.repo_key != repo_key {
                continue;
            }
        }
        let _guard = deps.locks.guard(&record.repo_key).await;

        match record.lifecycle {
            Lifecycle::Claimed => {
                skipped.push(PruneSkip {
                    id: record.worktree_id,
                    reason: "claimed".into(),
                });
                continue;
            }
            Lifecycle::Landing => {
                skipped.push(PruneSkip {
                    id: record.worktree_id,
                    reason: "landing".into(),
                });
                continue;
            }
            Lifecycle::LandBlocked => {
                skipped.push(PruneSkip {
                    id: record.worktree_id,
                    reason: "land-blocked".into(),
                });
                continue;
            }
            Lifecycle::Active | Lifecycle::Orphaned => {}
        }

        let repo = Path::new(&record.repo_path);
        let wt = Path::new(&record.path);

        if !wt.is_dir() {
            // Reconcile after a manual removal: the record is stale. Unlock
            // first — the creation-time lock also shields the stale admin
            // entry from `git worktree prune`.
            if !req.dry_run {
                if repo.is_dir() {
                    ops::worktree_unlock(repo, wt, t).await;
                    touched_repos.insert(record.repo_path.clone());
                }
                remove_record_and_emit(deps, &record, false).await?;
            }
            pruned.push(record.worktree_id.clone());
            continue;
        }

        let age_ms = now.saturating_sub(record.updated_at);
        if age_ms < expire_ms {
            skipped.push(PruneSkip {
                id: record.worktree_id,
                reason: "not expired".into(),
            });
            continue;
        }
        let st = match ops::status(wt, t).await {
            Ok(st) => st,
            Err(e) => {
                skipped.push(PruneSkip {
                    id: record.worktree_id,
                    reason: format!("status failed: {e}"),
                });
                continue;
            }
        };
        if !st.clean() {
            skipped.push(PruneSkip {
                id: record.worktree_id,
                reason: "dirty".into(),
            });
            continue;
        }
        let (_, ahead) = ops::ahead_behind(wt, &record.base_sha, t).await?;
        if ahead > 0 {
            skipped.push(PruneSkip {
                id: record.worktree_id,
                reason: "unmerged work".into(),
            });
            continue;
        }

        if !req.dry_run {
            ops::worktree_unlock(repo, wt, t).await;
            if let Err(e) = ops::worktree_remove(repo, wt, true, t).await {
                skipped.push(PruneSkip {
                    id: record.worktree_id,
                    reason: format!("remove failed: {e}"),
                });
                continue;
            }
            let branch_deleted = ops::branch_delete(repo, &record.branch, false, t).await;
            touched_repos.insert(record.repo_path.clone());
            remove_record_and_emit(deps, &record, branch_deleted).await?;
        }
        pruned.push(record.worktree_id.clone());
    }

    for repo_path in touched_repos {
        ops::worktree_prune(Path::new(&repo_path), t).await;
    }

    Ok(Response { pruned, skipped })
}

async fn remove_record_and_emit(
    deps: &Deps,
    record: &crate::types::WorktreeRecord,
    branch_deleted: bool,
) -> Result<(), WError> {
    state::delete_record(deps.state.as_ref(), &record.worktree_id).await?;
    state::clear_active_job_id(deps.state.as_ref(), &record.worktree_id).await?;
    deps.emitter
        .emit(
            EventKind::Removed,
            EventCtx {
                repo_path: &record.repo_path,
                worktree_id: &record.worktree_id,
                session_id: record.session_id.as_deref(),
            },
            &RemovedEvent {
                worktree_id: record.worktree_id.clone(),
                repo_path: record.repo_path.clone(),
                branch: record.branch.clone(),
                branch_deleted,
                timestamp: now_ms(),
            },
        )
        .await;
    Ok(())
}
