//! `worktree::land` — validate, persist the job, and enqueue it onto the
//! per-repo FIFO land queue. The engine groups by the payload's top-level
//! `repo_key` (see the README `queue_configs` block).

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{codes, WError};
use crate::functions::create::{require_dir, require_record};
use crate::functions::Deps;
use crate::git::{ops, validate_ref_name};
use crate::ids;
use crate::land;
use crate::state;
use crate::types::{now_ms, Lifecycle};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// The worktree to land.
    pub worktree_id: String,
    /// The branch to fast-forward with the worktree's commits.
    pub target_branch: String,
    /// Optional test command run in the worktree via `shell::exec` before
    /// merging (the test gate).
    #[serde(default)]
    pub test_cmd: Option<String>,
    /// Abort a conflicted rebase left by a previous land and start over.
    #[serde(default)]
    pub force_restart: bool,
    /// Keep the worktree and branch after a successful land.
    #[serde(default)]
    pub keep: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The queued land job.
    pub job_id: String,
    /// True when the job was enqueued.
    pub queued: bool,
}

pub async fn handle(deps: &Deps, req: Request) -> Result<Response, WError> {
    let cfg = deps.cfg().await;
    let t = cfg.git_timeout_ms;
    validate_ref_name("target_branch", &req.target_branch)?;

    let record = require_record(deps, &req.worktree_id).await?;
    let _guard = deps.locks.guard(&record.repo_key).await;
    let mut record = require_record(deps, &req.worktree_id).await?;
    require_dir(&record)?;

    if record.branch == req.target_branch {
        return Err(WError::new(
            codes::INVALID_REQUEST,
            "target_branch must differ from the worktree's own branch",
        ));
    }

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
                    "land job {active:?} is already queued or running for worktree {}",
                    record.worktree_id
                ),
            ));
        }
    }

    let wt = Path::new(&record.path);
    if ops::is_rebase_in_progress(wt, t).await? && !req.force_restart {
        return Err(WError::new(
            codes::LAND_CONFLICTED,
            format!(
                "worktree {} has a rebase in progress from a previous land; resolve it \
                 in place (then rerun worktree::land) or pass force_restart to abort \
                 and start over",
                record.worktree_id
            ),
        ));
    }

    let repo = Path::new(&record.repo_path);
    ops::rev_parse(repo, &req.target_branch, t).await?;

    let job_id = ids::mint_job_id();
    let job = land::new_job(
        job_id.clone(),
        &record,
        req.target_branch,
        req.test_cmd.filter(|c| !c.trim().is_empty()),
        req.force_restart,
        req.keep,
    );
    state::put_job(deps.state.as_ref(), &job).await?;
    state::set_active_job_id(deps.state.as_ref(), &record.worktree_id, &job_id).await?;
    let previous_lifecycle = record.lifecycle;
    record.lifecycle = Lifecycle::Landing;
    record.updated_at = now_ms();
    state::put_record(deps.state.as_ref(), &record).await?;

    let enqueue = deps
        .enqueuer
        .enqueue(
            &cfg.land_queue,
            "worktree::land-step",
            json!({ "job_id": job_id, "repo_key": record.repo_key }),
        )
        .await;
    if let Err(e) = enqueue {
        state::clear_active_job_id(deps.state.as_ref(), &record.worktree_id).await?;
        record.lifecycle = previous_lifecycle;
        record.updated_at = now_ms();
        state::put_record(deps.state.as_ref(), &record).await?;
        return Err(e);
    }

    Ok(Response {
        job_id,
        queued: true,
    })
}
