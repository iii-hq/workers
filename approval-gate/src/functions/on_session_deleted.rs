//! `approval::on-session-deleted` — bound to session-manager's
//! `session::deleted` trigger type. Purges the session's settings record
//! and every pending record (the cascade the prior deployment lacked).

use schemars::JsonSchema;
use serde::Deserialize;

use super::{purge, Deps};
use crate::settings;
use crate::types::EventAck;

/// `session::deleted` payload (only the field we read).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SessionDeletedEvent {
    pub session_id: String,
}

pub async fn handle(
    deps: &Deps,
    event: SessionDeletedEvent,
) -> Result<EventAck, crate::error::ApprovalError> {
    if let Err(e) = settings::clear(deps.iii.as_ref(), &event.session_id).await {
        tracing::warn!(session_id = %event.session_id, error = %e, "settings purge failed");
    }
    let purged = purge::purge_matching(deps, |r| r.session_id == event.session_id).await;
    tracing::info!(session_id = %event.session_id, purged, "session deleted: approval records purged");
    Ok(EventAck { ok: true })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::pending::PENDING_SCOPE;
    use crate::settings::SETTINGS_SCOPE;
    use crate::testkit::{log_snapshot, state_get, state_set, with_stack, BootOpts};

    async fn seed_pending(iii: &iii_sdk::IIIClient, sid: &str, cid: &str) {
        state_set(
            iii,
            PENDING_SCOPE,
            &format!("{sid}/{cid}"),
            json!({
                "session_id": sid,
                "turn_id": "t_1",
                "function_call_id": cid,
                "function_id": "shell::run",
                "pending_at": 1,
            }),
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn purges_settings_and_only_the_sessions_pending_records() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            state_set(&stack.iii, SETTINGS_SCOPE, "s_1", json!({ "mode": "auto" })).await;
            seed_pending(&stack.iii, "s_1", "c_1").await;
            seed_pending(&stack.iii, "s_1", "c_2").await;
            seed_pending(&stack.iii, "s_2", "c_3").await;

            handle(
                &stack.deps,
                SessionDeletedEvent {
                    session_id: "s_1".into(),
                },
            )
            .await
            .unwrap();

            assert!(state_get(&stack.iii, SETTINGS_SCOPE, "s_1").await.is_null());
            assert!(state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1")
                .await
                .is_null());
            assert!(state_get(&stack.iii, PENDING_SCOPE, "s_1/c_2")
                .await
                .is_null());
            assert!(!state_get(&stack.iii, PENDING_SCOPE, "s_2/c_3")
                .await
                .is_null());

            let events = log_snapshot(&stack.resolved);
            assert_eq!(events.len(), 2);
            assert!(events.iter().all(|e| e["outcome"] == json!("aborted")));
        })
        .await;
    }
}
