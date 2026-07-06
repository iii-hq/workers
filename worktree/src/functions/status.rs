//! `worktree::status` — git status for one managed worktree, plus the
//! shared status builder used by `get` and `list`.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::WError;
use crate::functions::create::{require_dir, require_record};
use crate::functions::Deps;
use crate::git::ops;
use crate::types::{Lifecycle, WorktreeRecord, WorktreeStatus};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// The worktree to inspect.
    pub worktree_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The inspected worktree.
    pub worktree_id: String,
    /// The branch checked out in the worktree.
    pub branch: String,
    /// Current lifecycle.
    pub lifecycle: Lifecycle,
    /// Git status details.
    #[serde(flatten)]
    pub status: WorktreeStatus,
}

pub async fn handle(deps: &Deps, req: Request) -> Result<Response, WError> {
    let cfg = deps.cfg().await;
    let record = require_record(deps, &req.worktree_id).await?;
    require_dir(&record)?;
    let status = build_status(&record, cfg.git_timeout_ms).await?;
    Ok(Response {
        worktree_id: record.worktree_id,
        branch: record.branch,
        lifecycle: record.lifecycle,
        status,
    })
}

/// Compute the full status summary for one worktree. Ahead/behind and
/// `unpushed` come from the upstream when one is set, else from `base_sha`.
pub async fn build_status(
    record: &WorktreeRecord,
    git_timeout_ms: u64,
) -> Result<WorktreeStatus, WError> {
    let wt = Path::new(&record.path);
    let t = git_timeout_ms;
    let st = ops::status(wt, t).await?;
    let in_rebase = ops::is_rebase_in_progress(wt, t).await?;
    let head_sha = ops::rev_parse(wt, "HEAD", t).await?;
    let (behind_base, ahead_base) = ops::ahead_behind(wt, &record.base_sha, t).await?;
    let (ahead, behind, unpushed) = if st.has_upstream {
        (st.ahead, st.behind, st.ahead)
    } else {
        (ahead_base, behind_base, ahead_base)
    };
    let diffstat = ops::diffstat(wt, &record.base_sha, t).await?;
    Ok(WorktreeStatus {
        clean: st.clean(),
        ahead,
        behind,
        staged: st.staged,
        unstaged: st.unstaged,
        untracked: st.untracked,
        conflicted: st.conflicted,
        diffstat,
        unpushed,
        in_rebase,
        head_sha,
    })
}
