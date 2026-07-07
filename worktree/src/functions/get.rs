//! `worktree::get` — one managed worktree, status included when computable.

use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;

use crate::error::WError;
use crate::functions::create::require_record;
use crate::functions::status::{build_status, TargetCache};
use crate::functions::Deps;
use crate::types::{Lifecycle, WorktreeInfo};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// The worktree to fetch.
    pub worktree_id: String,
}

pub type Response = WorktreeInfo;

pub async fn handle(deps: &Deps, req: Request) -> Result<Response, WError> {
    let cfg = deps.cfg().await;
    let record = require_record(deps, &req.worktree_id).await?;
    let dir_exists = Path::new(&record.path).is_dir();
    let lifecycle = if dir_exists {
        record.lifecycle
    } else {
        Lifecycle::Orphaned
    };
    let mut info = WorktreeInfo::from_record(&record, lifecycle);
    if dir_exists {
        match build_status(&record, cfg.git_timeout_ms, &mut TargetCache::new()).await {
            Ok(status) => info.status = Some(status),
            Err(e) => {
                tracing::warn!(worktree_id = %record.worktree_id, error = %e, "status failed")
            }
        }
    }
    Ok(info)
}
