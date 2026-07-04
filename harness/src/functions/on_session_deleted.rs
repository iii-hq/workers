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
    let dropped = deps.subscriptions.take_session(&event.session_id);
    let tracked = dropped.len() as u64;
    if tracked > 0 {
        for (_sub_id, trigger_id) in dropped {
            if let Some(trigger_id) = trigger_id {
                crate::functions::subscribe::unregister_engine_trigger(deps, &trigger_id).await;
            }
        }
        tracing::info!(
            session_id = %event.session_id,
            removed = tracked,
            "session deleted: ephemeral subscriptions dropped"
        );
    }
    let cfg = deps.cfg().await;
    crate::filesystem_grants::purge(&deps.iii, &event.session_id, cfg.session_timeout_ms).await?;
    // Family comm log: keyed by root session id, so this only matches when the
    // deleted session IS a family root; child deletions no-op harmlessly.
    if let Err(e) = crate::state::state_delete(
        &deps.iii,
        crate::comm::COMM_LOG_SCOPE,
        &event.session_id,
        cfg.session_timeout_ms,
    )
    .await
    {
        tracing::warn!(session_id = %event.session_id, error = %e, "comm log cleanup failed");
    }
    // Durable complement: bindings registered before a harness restart are no
    // longer in the in-memory registry but still fire engine-side — sweep them
    // by their owner stamp so deleting a chat fully tears down its wiring.
    let swept = crate::subscriptions::reconcile::sweep_owner(deps, &event.session_id).await;
    Ok(SessionDeletedAck {
        ok: true,
        removed: tracked + swept as u64,
    })
}
