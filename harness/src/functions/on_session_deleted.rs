//! `harness::on-session-deleted` — drop a deleted session's ephemeral
//! subscriptions. Bound to session-manager's `session::deleted` trigger.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;

/// `session::deleted` payload (only the field we read).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SessionDeletedEvent {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionDeletedAck {
    pub ok: bool,
    pub removed: u64,
}

pub async fn handle(
    deps: &Deps,
    event: SessionDeletedEvent,
) -> Result<SessionDeletedAck, HarnessError> {
    // One durable sweep: every binding this session owned, unregistered
    // engine-side and deleted from the store. Before the store existed this
    // needed two passes — an in-memory one for bindings this process knew
    // about, and an owner-stamp scan for everything registered before the last
    // restart — and the second pass only ever saw what the first had lost.
    let swept = super::teardown::cleanup_session(deps, &event.session_id).await;
    if swept > 0 {
        tracing::info!(
            session_id = %event.session_id,
            removed = swept,
            "session deleted: trigger bindings dropped"
        );
    }

    let cfg = deps.cfg().await;
    crate::filesystem_grants::purge(&deps.iii, &event.session_id, cfg.session_timeout_ms).await?;
    crate::budget::purge(deps, &event.session_id, cfg.session_timeout_ms).await?;
    crate::context_snapshot::delete(&deps.iii, &event.session_id, cfg.session_timeout_ms).await?;
    crate::usage_report::delete(&deps.iii, &event.session_id, cfg.session_timeout_ms).await?;
    // Last, so a failed purge above keeps the record for the handler's retry.
    // Under the step guards, as delete-session-tree's erase: a running step
    // writes back the record it holds when it ends, so delete after that write
    // rather than have it re-create the record.
    let _activity = deps.turn_activity.guard(&event.session_id).await;
    let _lock = deps.locks.guard(&event.session_id).await;
    crate::state::delete_turn(&deps.iii, &event.session_id, cfg.session_timeout_ms).await?;
    Ok(SessionDeletedAck {
        ok: true,
        removed: swept,
    })
}
