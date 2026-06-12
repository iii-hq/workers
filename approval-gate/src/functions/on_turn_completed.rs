//! `approval::on_turn_completed` — bound to the harness's
//! `harness::turn_completed` trigger type. Purges the turn's pending
//! records when it goes terminal (covers `harness::stop` cancellation
//! cascades and failed turns). A `completed` turn has no live holds by
//! construction — the purge is then a harmless stale-record cleanup.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::{purge, Deps};

/// `harness::turn_completed` payload (only the field we read).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TurnCompletedEvent {
    pub turn_id: String,
}

pub async fn handle(
    deps: &Deps,
    event: TurnCompletedEvent,
) -> Result<Value, crate::error::ApprovalError> {
    let purged = purge::purge_matching(deps, |r| r.turn_id == event.turn_id).await;
    if purged > 0 {
        tracing::info!(turn_id = %event.turn_id, purged, "terminal turn: pending approvals purged");
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::config::WorkerConfig;
    use crate::events::RecordingSink;
    use crate::gate_config::shared_defaults;
    use crate::pending::PENDING_SCOPE;
    use crate::testkit::FakeBus;
    use crate::types::ResolvedOutcome;

    #[tokio::test]
    async fn purges_by_turn_id_and_emits_aborted() {
        let bus = Arc::new(FakeBus::new());
        let state = bus.with_memory_state();
        let sink = Arc::new(RecordingSink::new());
        let deps = Arc::new(Deps {
            bus,
            sink: sink.clone(),
            defaults: shared_defaults(),
            cfg: Arc::new(WorkerConfig::default()),
        });

        for (sid, cid, tid) in [("s_1", "c_1", "t_1"), ("s_1", "c_2", "t_2")] {
            state.seed(
                PENDING_SCOPE,
                &format!("{sid}/{cid}"),
                json!({
                    "session_id": sid,
                    "turn_id": tid,
                    "function_call_id": cid,
                    "function_id": "shell::run",
                    "pending_at": 1,
                    "expires_at": 2,
                }),
            );
        }

        handle(
            &deps,
            TurnCompletedEvent {
                turn_id: "t_1".into(),
            },
        )
        .await
        .unwrap();

        assert!(state.peek(PENDING_SCOPE, "s_1/c_1").is_none());
        assert!(state.peek(PENDING_SCOPE, "s_1/c_2").is_some());
        let events = sink.resolved_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, ResolvedOutcome::Aborted);
        assert_eq!(events[0].turn_id, "t_1");
    }
}
