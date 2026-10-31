//! `approval::on-turn-completed` — the target of this worker's observer of
//! `harness::on-turn-completed`. Purges the turn's pending
//! records AND its filesystem-access-watch attempt counters when it goes terminal
//! (covers `harness::stop` cancellation cascades and failed turns). A
//! `completed` turn has no live holds by construction — the purge is then
//! a harmless stale-record cleanup.

use schemars::JsonSchema;
use serde::Deserialize;

use super::{purge, Deps};
use crate::filesystem_access_state;
use crate::types::EventAck;

/// `harness::on-turn-completed` event (only the field we read).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TurnCompletedEvent {
    pub turn_id: String,
}

pub async fn handle(
    deps: &Deps,
    event: TurnCompletedEvent,
) -> Result<EventAck, crate::error::ApprovalError> {
    let purged = purge::purge_matching(deps, |r| r.turn_id == event.turn_id).await;
    let purged_attempts =
        filesystem_access_state::purge_matching(deps.iii.as_ref(), |r| r.turn_id == event.turn_id)
            .await;
    if purged > 0 || purged_attempts > 0 {
        tracing::info!(
            turn_id = %event.turn_id,
            purged,
            purged_attempts,
            "terminal turn: pending approvals + filesystem-access-watch attempts purged"
        );
    }
    Ok(EventAck { ok: true })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::pending::PENDING_SCOPE;
    use crate::testkit::{
        log_snapshot, settle, state_get, state_set, wait_for, with_stack, BootOpts,
    };

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
                        "kind": "function",
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
            // Delivery to observers does not wait for them.
            assert!(
                wait_for(3_000, || !log_snapshot(&stack.resolved).is_empty()).await,
                "pending_resolved should reach the recorder"
            );
            settle().await;
            let events = log_snapshot(&stack.resolved);
            assert_eq!(events.len(), 1);
            assert_eq!(events[0]["outcome"], json!("aborted"));
            assert_eq!(events[0]["turn_id"], json!("t_1"));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn purges_filesystem_access_watch_attempt_counters_by_turn_id() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            for (sid, cid, tid) in [("s_1", "c_1", "t_1"), ("s_1", "c_2", "t_2")] {
                state_set(
                    &stack.iii,
                    filesystem_access_state::ATTEMPTS_SCOPE,
                    &format!("{sid}/{cid}"),
                    json!({
                        "session_id": sid,
                        "function_call_id": cid,
                        "turn_id": tid,
                        "count": 1,
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

            assert!(state_get(
                &stack.iii,
                filesystem_access_state::ATTEMPTS_SCOPE,
                "s_1/c_1"
            )
            .await
            .is_null());
            assert!(!state_get(
                &stack.iii,
                filesystem_access_state::ATTEMPTS_SCOPE,
                "s_1/c_2"
            )
            .await
            .is_null());
        })
        .await;
    }
}
