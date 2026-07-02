//! `worktree::claim` — take session ownership of a worktree.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{codes, WError};
use crate::events::{ClaimedEvent, EventCtx, EventKind};
use crate::functions::create::require_record;
use crate::functions::Deps;
use crate::state;
use crate::types::{now_ms, Lifecycle};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// The worktree to claim.
    pub worktree_id: String,
    /// The claiming session.
    pub session_id: String,
    /// Take the claim even when another session holds it.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// True when the claim is now held by `session_id`.
    pub claimed: bool,
    /// The claimed worktree.
    pub worktree_id: String,
    /// The claiming session.
    pub session_id: String,
    /// The session that held the claim before, when taken over with force.
    pub previous_session_id: Option<String>,
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
    // Re-read under the lock so concurrent claims serialize correctly.
    record = require_record(deps, &req.worktree_id).await?;

    let previous = record.session_id.clone();
    if let Some(owner) = &previous {
        if owner != &req.session_id && !req.force {
            return Err(WError::new(
                codes::ALREADY_CLAIMED,
                format!(
                    "worktree {} is claimed by session {owner:?}; pass force to take over",
                    record.worktree_id
                ),
            ));
        }
    }

    let previous_session_id = previous.filter(|p| p != &req.session_id);
    record.session_id = Some(req.session_id.clone());
    // Ownership changes, but a land in flight (or a prior block) owns the
    // lifecycle: only a plain unclaimed/claimed worktree flips to `claimed`.
    if matches!(record.lifecycle, Lifecycle::Active | Lifecycle::Claimed) {
        record.lifecycle = Lifecycle::Claimed;
    }
    record.updated_at = now_ms();
    state::put_record(deps.state.as_ref(), &record).await?;

    deps.emitter
        .emit(
            EventKind::Claimed,
            EventCtx {
                repo_path: &record.repo_path,
                worktree_id: &record.worktree_id,
                session_id: Some(&req.session_id),
            },
            &ClaimedEvent {
                worktree_id: record.worktree_id.clone(),
                repo_path: record.repo_path.clone(),
                session_id: req.session_id.clone(),
                previous_session_id: previous_session_id.clone(),
                timestamp: now_ms(),
            },
        )
        .await;

    Ok(Response {
        claimed: true,
        worktree_id: record.worktree_id,
        session_id: req.session_id,
        previous_session_id,
    })
}
