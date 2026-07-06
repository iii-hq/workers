//! `worktree::remove` — remove a managed worktree, guarding uncommitted
//! and unmerged work unless forced.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{codes, WError};
use crate::events::{EventCtx, EventKind, RemovedEvent};
use crate::functions::create::require_record;
use crate::functions::Deps;
use crate::git::ops;
use crate::state;
use crate::types::now_ms;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// The worktree to remove.
    pub worktree_id: String,
    /// Remove even when dirty or carrying unmerged commits.
    #[serde(default)]
    pub force: bool,
    /// Also delete the worktree's branch.
    #[serde(default)]
    pub delete_branch: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// True when the worktree (and its record) are gone.
    pub removed: bool,
    /// True when the branch was deleted too.
    pub branch_deleted: bool,
}

pub async fn handle(deps: &Deps, req: Request) -> Result<Response, WError> {
    let cfg = deps.cfg().await;
    let t = cfg.git_timeout_ms;
    let record = require_record(deps, &req.worktree_id).await?;
    let _guard = deps.locks.guard(&record.repo_key).await;
    let record = require_record(deps, &req.worktree_id).await?;

    // Refuse while a land job is live: removing the worktree out from under a
    // rebase/test/merge in flight would corrupt the land. Checked under the
    // lock so it cannot race a land that is being enqueued.
    if let Some(active) = state::get_active_job_id(deps.state.as_ref(), &record.worktree_id).await?
    {
        let live = match state::get_job(deps.state.as_ref(), &active).await? {
            Some(job) => !job.done,
            None => false,
        };
        if live {
            return Err(WError::new(
                codes::LAND_IN_PROGRESS,
                format!(
                    "worktree {} has an active land job {active:?}; wait for it to finish \
                     or resolve it before removing",
                    record.worktree_id
                ),
            ));
        }
    }

    let repo = Path::new(&record.repo_path);
    let wt = Path::new(&record.path);
    let repo_available = repo.is_dir();

    if wt.is_dir() && repo_available {
        if !req.force {
            let st = ops::status(wt, t).await?;
            if !st.clean() {
                return Err(WError::new(
                    codes::DIRTY,
                    format!(
                        "worktree {} has uncommitted changes; pass force to discard them",
                        record.worktree_id
                    ),
                ));
            }
            let (_, ahead) = ops::ahead_behind(wt, &record.base_sha, t).await?;
            if ahead > 0 {
                return Err(WError::new(
                    codes::UNMERGED_WORK,
                    format!(
                        "worktree {} has {ahead} commit(s) past its base that are not \
                         landed; land them or pass force to drop them",
                        record.worktree_id
                    ),
                ));
            }
        }
        ops::worktree_unlock(repo, wt, t).await;
        ops::worktree_remove(repo, wt, true, t).await?;
    } else if repo_available {
        // Directory already gone: clean up stale admin metadata.
        ops::worktree_prune(repo, t).await;
    }

    let mut branch_deleted = false;
    if req.delete_branch && repo_available {
        branch_deleted = ops::branch_delete(repo, &record.branch, req.force, t).await;
    }

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

    Ok(Response {
        removed: true,
        branch_deleted,
    })
}
