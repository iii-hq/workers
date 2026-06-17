//! Shared purge loop for the two terminal-cleanup handlers
//! (`on_session_deleted`, `on_turn_completed`): delete every matching
//! pending record through the emit gate and fire `pending_resolved`
//! with `outcome: "aborted"` per record actually deleted.

use super::Deps;
use crate::pending;
use crate::types::{now_ms, PendingApprovalRecord, PendingResolvedEvent, ResolvedOutcome};

pub async fn purge_matching<F>(deps: &Deps, predicate: F) -> usize
where
    F: Fn(&PendingApprovalRecord) -> bool,
{
    let cfg = deps.config().await;
    let iii = deps.iii.as_ref();
    let records = match pending::list_all(iii, cfg.state_timeout_ms).await {
        Ok(records) => records,
        Err(e) => {
            tracing::warn!(error = %e, "purge: pending list failed; sweep will retry");
            return 0;
        }
    };

    let mut purged = 0usize;
    for record in records.into_iter().filter(|r| predicate(r)) {
        match pending::delete_with_gate(
            iii,
            &record.session_id,
            &record.function_call_id,
            cfg.state_timeout_ms,
        )
        .await
        {
            Ok(Some(deleted)) => {
                purged += 1;
                deps.sink
                    .pending_resolved(&PendingResolvedEvent {
                        session_id: deleted.session_id,
                        turn_id: deleted.turn_id,
                        function_call_id: deleted.function_call_id,
                        function_id: deleted.function_id,
                        outcome: ResolvedOutcome::Aborted,
                        reason: None,
                        session_metadata: deleted.session_metadata,
                        resolved_at: now_ms(),
                    })
                    .await;
            }
            // Already deleted by a racing path — that path emitted.
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    session_id = %record.session_id,
                    function_call_id = %record.function_call_id,
                    error = %e,
                    "purge: delete failed; sweep will collect the record"
                );
            }
        }
    }
    purged
}
