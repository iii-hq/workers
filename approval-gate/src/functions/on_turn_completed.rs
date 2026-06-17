//! `approval::on-turn-completed` — bound to the harness's
//! `harness::turn-completed` trigger type. Purges the turn's pending
//! records when it goes terminal (covers `harness::stop` cancellation
//! cascades and failed turns). A `completed` turn has no live holds by
//! construction — the purge is then a harmless stale-record cleanup.

use schemars::JsonSchema;
use serde::Deserialize;

use super::{purge, Deps};
use crate::types::EventAck;

/// `harness::turn-completed` payload (only the field we read).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TurnCompletedEvent {
    pub turn_id: String,
}

pub async fn handle(
    deps: &Deps,
    event: TurnCompletedEvent,
) -> Result<EventAck, crate::error::ApprovalError> {
    let purged = purge::purge_matching(deps, |r| r.turn_id == event.turn_id).await;
    if purged > 0 {
        tracing::info!(turn_id = %event.turn_id, purged, "terminal turn: pending approvals purged");
    }
    Ok(EventAck { ok: true })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::pending::PENDING_SCOPE;
    use crate::testkit::{log_snapshot, state_get, state_set, with_stack, BootOpts};

    #[tokio::test(flavor = "multi_thread")]
    async fn purges_by_turn_id_and_emits_aborted() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            for (sid, cid, tid) in [("s_1", "c_1", "t_1"), ("s_1", "c_2", "t_2")] {
                state_set(
                    &stack.iii,
                    PENDING_SCOPE,
                    &format!("{sid}/{cid}"),
                    json!({
                        "session_id": sid,
                        "turn_id": tid,
                        "function_call_id": cid,
                        "function_id": "shell::run",
                        "pending_at": 1,
                    }),
                )
                .await;
            }

            handle(
                &stack.deps,
                TurnCompletedEvent {
                    turn_id: "t_1".into(),
                },
            )
            .await
            .unwrap();

            assert!(state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1")
                .await
                .is_null());
            assert!(!state_get(&stack.iii, PENDING_SCOPE, "s_1/c_2")
                .await
                .is_null());
            let events = log_snapshot(&stack.resolved);
            assert_eq!(events.len(), 1);
            assert_eq!(events[0]["outcome"], json!("aborted"));
            assert_eq!(events[0]["turn_id"], json!("t_1"));
        })
        .await;
    }
}
