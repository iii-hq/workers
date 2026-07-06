//! `worktree::validate` — the console-picker control-plane surface,
//! mirroring `shell::workspace::validate`. Reconciles a record whose
//! directory is gone to `orphaned`.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::WError;
use crate::functions::Deps;
use crate::git::{ops, validate_abs_path};
use crate::state;
use crate::types::{now_ms, Lifecycle};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// Absolute path to validate.
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// True when the path is a live, managed worktree.
    pub valid: bool,
    /// The managing record's id, when one exists.
    pub worktree_id: Option<String>,
    /// True when a record manages this path.
    pub managed: bool,
    /// Why the path is not valid.
    pub reason: Option<String>,
}

pub async fn handle(deps: &Deps, req: Request) -> Result<Response, WError> {
    let cfg = deps.cfg().await;
    let t = cfg.git_timeout_ms;
    let path = validate_abs_path("path", &req.path)?;
    let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());

    let records = state::list_records(deps.state.as_ref()).await?;
    let matched = records.into_iter().find(|r| {
        let record_path = Path::new(&r.path);
        record_path == path
            || record_path == canonical
            || std::fs::canonicalize(record_path)
                .map(|c| c == canonical)
                .unwrap_or(false)
    });

    if let Some(mut record) = matched {
        let worktree_id = record.worktree_id.clone();
        if !Path::new(&record.path).is_dir() {
            if record.lifecycle != Lifecycle::Orphaned {
                record.lifecycle = Lifecycle::Orphaned;
                record.updated_at = now_ms();
                state::put_record(deps.state.as_ref(), &record).await?;
            }
            return Ok(Response {
                valid: false,
                worktree_id: Some(worktree_id),
                managed: true,
                reason: Some("worktree directory is missing (orphaned)".to_string()),
            });
        }
        return match ops::git_common_dir(&path, t).await {
            Ok(_) => Ok(Response {
                valid: true,
                worktree_id: Some(worktree_id),
                managed: true,
                reason: None,
            }),
            Err(_) => Ok(Response {
                valid: false,
                worktree_id: Some(worktree_id),
                managed: true,
                reason: Some("path is no longer a git worktree".to_string()),
            }),
        };
    }

    if path.is_dir() && ops::git_common_dir(&path, t).await.is_ok() {
        return Ok(Response {
            valid: false,
            worktree_id: None,
            managed: false,
            reason: Some(
                "unmanaged worktree: not created by this worker and never auto-adopted".to_string(),
            ),
        });
    }

    Ok(Response {
        valid: false,
        worktree_id: None,
        managed: false,
        reason: Some("path is missing or not a git worktree".to_string()),
    })
}
