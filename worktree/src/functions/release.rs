//! `worktree::release` — release a session's claim.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{codes, WError};
use crate::events::{EventCtx, EventKind, ReleasedEvent};
use crate::functions::create::require_record;
use crate::functions::Deps;
use crate::state;
use crate::types::{now_ms, Lifecycle};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// The worktree to release.
    pub worktree_id: String,
    /// The releasing session (must hold the claim unless force is set).
    pub session_id: String,
    /// Release even when another session holds the claim.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// True when the worktree is now unclaimed.
    pub released: bool,
    /// The released worktree.
    pub worktree_id: String,
}

pub async fn handle(deps: &Deps, req: Request) -> Result<Response, WError> {
    if req.session_id.trim().is_empty() {
        return Err(WError::new(
            codes::INVALID_REQUEST,
            "session_id must be non-empty",
        ));
    }
    let mut record = require_record(deps, &req.worktree_id).await?;
    let _guard = deps.locks.guard(&record.repo_key).await;
    record = require_record(deps, &req.worktree_id).await?;

    match &record.session_id {
        Some(owner) if owner == &req.session_id || req.force => {}
        Some(owner) => {
            return Err(WError::new(
                codes::CLAIM_MISMATCH,
                format!(
                    "worktree {} is claimed by session {owner:?}, not {:?}; pass force to override",
                    record.worktree_id, req.session_id
                ),
            ));
        }
        None => {
            // Releasing an unclaimed worktree is a no-op.
            return Ok(Response {
                released: true,
                worktree_id: record.worktree_id,
            });
        }
    }

    record.session_id = None;
    if record.lifecycle == Lifecycle::Claimed {
        record.lifecycle = Lifecycle::Active;
    }
    record.updated_at = now_ms();
    state::put_record(deps.state.as_ref(), &record).await?;

    deps.emitter
        .emit(
            EventKind::Released,
            EventCtx {
                repo_path: &record.repo_path,
                worktree_id: &record.worktree_id,
                session_id: Some(&req.session_id),
            },
            &ReleasedEvent {
                worktree_id: record.worktree_id.clone(),
                repo_path: record.repo_path.clone(),
                session_id: req.session_id.clone(),
                timestamp: now_ms(),
            },
        )
        .await;

    Ok(Response {
        released: true,
        worktree_id: record.worktree_id,
    })
}
