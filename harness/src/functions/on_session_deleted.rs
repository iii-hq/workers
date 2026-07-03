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
    let removed = dropped.len() as u64;
    if removed > 0 {
        for (_sub_id, trigger_id) in dropped {
            if let Some(trigger_id) = trigger_id {
                crate::functions::subscribe::unregister_engine_trigger(deps, &trigger_id).await;
            }
        }
        tracing::info!(
            session_id = %event.session_id,
            removed,
            "session deleted: ephemeral subscriptions dropped"
        );
    }
    let cfg = deps.cfg().await;
    crate::filesystem_grants::purge(&deps.iii, &event.session_id, cfg.session_timeout_ms).await?;
    Ok(SessionDeletedAck { ok: true, removed })
}
