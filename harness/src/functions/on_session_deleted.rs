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
    deps.subscriptions.forget_lineage(&event.session_id);
    let removed = super::teardown::cleanup_session(deps, &event.session_id).await;
    let cfg = deps.cfg().await;
    crate::filesystem_grants::purge(&deps.iii, &event.session_id, cfg.session_timeout_ms).await?;
    crate::budget::purge(deps, &event.session_id, cfg.session_timeout_ms).await?;
    Ok(SessionDeletedAck { ok: true, removed })
}
