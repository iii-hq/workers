//! `worktree::list` — the cross-agent registry view. Directories without
//! records are reported as unmanaged and never adopted.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::WError;
use crate::functions::status::{build_status, TargetCache};
use crate::functions::Deps;
use crate::git::{ops, validate_abs_path};
use crate::state;
use crate::types::{Lifecycle, WorktreeInfo};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct Request {
    /// Only worktrees of this repository.
    #[serde(default)]
    pub repo_path: Option<String>,
    /// Only worktrees claimed by this session.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Compute git status for each worktree (slower).
    #[serde(default)]
    pub include_status: bool,
}

/// A worktree of the repo that this worker does not manage.
#[derive(Debug, Serialize, JsonSchema)]
pub struct UnmanagedWorktree {
    /// Absolute path of the worktree.
    pub path: String,
    /// Checked-out branch ref, when not detached.
    pub branch: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// Managed worktrees, oldest first.
    pub worktrees: Vec<WorktreeInfo>,
    /// Non-primary worktrees of the repo without a record (only populated
    /// when repo_path is given). Never auto-adopted.
    pub unmanaged: Vec<UnmanagedWorktree>,
}

pub async fn handle(deps: &Deps, req: Request) -> Result<Response, WError> {
    let cfg = deps.cfg().await;
    let t = cfg.git_timeout_ms;

    let repo_filter = match &req.repo_path {
        Some(raw) => {
            let repo = validate_abs_path("repo_path", raw)?;
            Some((repo.clone(), ops::git_common_dir(&repo, t).await?))
        }
        None => None,
    };

    let mut worktrees = Vec::new();
    let mut targets = TargetCache::new();
    for record in state::list_records(deps.state.as_ref()).await? {
        if let Some((_, repo_key)) = &repo_filter {
            if &record.repo_key != repo_key {
                continue;
            }
        }
        if let Some(session_id) = &req.session_id {
            if record.session_id.as_ref() != Some(session_id) {
                continue;
            }
        }
        let dir_exists = Path::new(&record.path).is_dir();
        let lifecycle = if dir_exists {
            record.lifecycle
        } else {
            Lifecycle::Orphaned
        };
        let mut info = WorktreeInfo::from_record(&record, lifecycle);
        if req.include_status && dir_exists {
            match build_status(&record, t, &mut targets).await {
                Ok(status) => info.status = Some(status),
                Err(e) => {
                    tracing::warn!(worktree_id = %record.worktree_id, error = %e, "status failed")
                }
            }
        }
        worktrees.push(info);
    }

    let mut unmanaged = Vec::new();
    if let Some((repo, _)) = &repo_filter {
        let entries = ops::worktree_list(repo, t).await?;
        for entry in entries.into_iter().skip(1) {
            let managed = worktrees.iter().any(|w| w.path == entry.path);
            if !managed && !entry.bare {
                unmanaged.push(UnmanagedWorktree {
                    path: entry.path,
                    branch: entry.branch,
                });
            }
        }
    }

    Ok(Response {
        worktrees,
        unmanaged,
    })
}
