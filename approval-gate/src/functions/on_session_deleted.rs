//! `approval::on_session_deleted` — bound to session-manager's
//! `session::deleted` trigger type. Purges the session's settings record
//! and every pending record (the cascade the prior deployment lacked).

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::{purge, Deps};
use crate::settings;

/// `session::deleted` payload (only the field we read).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SessionDeletedEvent {
    pub session_id: String,
}

pub async fn handle(
    deps: &Deps,
    event: SessionDeletedEvent,
) -> Result<Value, crate::error::ApprovalError> {
    if let Err(e) = settings::clear(
        deps.bus.as_ref(),
        &event.session_id,
        deps.cfg.state_timeout_ms,
    )
    .await
    {
        tracing::warn!(session_id = %event.session_id, error = %e, "settings purge failed");
    }
    let purged = purge::purge_matching(deps, |r| r.session_id == event.session_id).await;
    tracing::info!(session_id = %event.session_id, purged, "session deleted: approval records purged");
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
    use crate::settings::SETTINGS_SCOPE;
    use crate::testkit::{FakeBus, MemoryState};
    use crate::types::ResolvedOutcome;

    fn seed_pending(state: &MemoryState, sid: &str, cid: &str) {
        state.seed(
            PENDING_SCOPE,
            &format!("{sid}/{cid}"),
            json!({
                "session_id": sid,
                "turn_id": "t_1",
                "function_call_id": cid,
                "function_id": "shell::run",
                "pending_at": 1,
                "expires_at": 2,
            }),
        );
    }

    #[tokio::test]
    async fn purges_settings_and_only_the_sessions_pending_records() {
        let bus = Arc::new(FakeBus::new());
        let state = bus.with_memory_state();
        let sink = Arc::new(RecordingSink::new());
        let deps = Arc::new(Deps {
            bus,
            sink: sink.clone(),
            defaults: shared_defaults(),
            cfg: Arc::new(WorkerConfig::default()),
        });

        state.seed(SETTINGS_SCOPE, "s_1", json!({ "mode": "auto" }));
        seed_pending(&state, "s_1", "c_1");
        seed_pending(&state, "s_1", "c_2");
        seed_pending(&state, "s_2", "c_3");

        handle(
            &deps,
            SessionDeletedEvent {
                session_id: "s_1".into(),
            },
        )
        .await
        .unwrap();

        assert!(state.peek(SETTINGS_SCOPE, "s_1").is_none());
        assert!(state.peek(PENDING_SCOPE, "s_1/c_1").is_none());
        assert!(state.peek(PENDING_SCOPE, "s_1/c_2").is_none());
        // Other sessions untouched.
        assert!(state.peek(PENDING_SCOPE, "s_2/c_3").is_some());

        let events = sink.resolved_events();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.outcome == ResolvedOutcome::Aborted));
    }
}
