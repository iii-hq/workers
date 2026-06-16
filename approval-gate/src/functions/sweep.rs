//! `approval::sweep` — the cron-bound GC backstop (approval-gate.md §
//! Sweep). For every record past `expires_at`: settle the held call with
//! a timeout denial, delete through the emit gate, fire
//! `pending_resolved { outcome: "timeout" }`. Also collects records
//! orphaned by a crash between resolve and delete — which is why no
//! delete path needs to be transactional.

use serde_json::{json, Value};

use super::Deps;
use crate::error::ApprovalError;
use crate::harness;
use crate::pending;
use crate::types::{
    now_ms, text_block, PendingApprovalRecord, PendingResolvedEvent, ResolvedOutcome,
};

pub async fn handle(deps: &Deps, _payload: Value) -> Result<Value, ApprovalError> {
    let iii = deps.iii.as_ref();
    let now = now_ms();

    let records = match pending::list_all(iii, deps.cfg.state_timeout_ms).await {
        Ok(records) => records,
        Err(e) => {
            tracing::warn!(error = %e, "sweep: pending list failed; retrying next tick");
            return Ok(json!({ "swept": 0 }));
        }
    };

    let mut swept = 0usize;
    for record in records.into_iter().filter(|r| r.expires_at <= now) {
        // Settle the call first — a no-op `{ resolved: false }` when the
        // harness's own pending sweep or another path got there already.
        // A transport failure is tolerated: the record is still deleted
        // (the GC contract — the inbox must stay O(live)), and the
        // harness sweep remains the backstop for the parked turn.
        let resolve_payload = timeout_resolve_payload(&record);
        if let Err(e) =
            harness::function_resolve(iii, resolve_payload, Some(deps.cfg.harness_timeout_ms)).await
        {
            tracing::warn!(
                session_id = %record.session_id,
                function_call_id = %record.function_call_id,
                error = %e,
                "sweep: harness resolve failed; deleting the expired record anyway"
            );
        }

        match pending::delete_with_gate(
            iii,
            &record.session_id,
            &record.function_call_id,
            deps.cfg.state_timeout_ms,
        )
        .await
        {
            Ok(Some(deleted)) => {
                swept += 1;
                deps.sink
                    .pending_resolved(&PendingResolvedEvent {
                        session_id: deleted.session_id,
                        turn_id: deleted.turn_id,
                        function_call_id: deleted.function_call_id,
                        function_id: deleted.function_id,
                        outcome: ResolvedOutcome::Timeout,
                        reason: None,
                        session_metadata: deleted.session_metadata,
                        resolved_at: now_ms(),
                    })
                    .await;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    session_id = %record.session_id,
                    function_call_id = %record.function_call_id,
                    error = %e,
                    "sweep: delete failed; retrying next tick"
                );
            }
        }
    }

    if swept > 0 {
        tracing::info!(swept, "sweep: expired pending approvals collected");
    }
    Ok(json!({ "swept": swept }))
}

/// Not a `DenialEnvelope` — nobody denied the call; no human decision
/// arrived before the deadline. The text and details still give the
/// model something to adapt to.
fn timeout_resolve_payload(record: &PendingApprovalRecord) -> Value {
    let window_ms = record.expires_at - record.pending_at;
    let text = format!(
        "Approval request for {} timed out: no human decision within {}ms. The call was not executed.",
        record.function_id, window_ms
    );
    json!({
        "session_id": record.session_id,
        "turn_id": record.turn_id,
        "function_call_id": record.function_call_id,
        "action": "deliver",
        "is_error": true,
        "content": [text_block(text)],
        "details": {
            "status": "timeout",
            "function_id": record.function_id,
            "pending_at": record.pending_at,
            "expires_at": record.expires_at,
        },
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::pending::PENDING_SCOPE;
    use crate::testkit::{log_snapshot, state_get, state_set, with_stack, BootOpts};
    use crate::types::now_ms;

    #[tokio::test(flavor = "multi_thread")]
    async fn sweeps_only_expired_records() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            state_set(
                &stack.iii,
                PENDING_SCOPE,
                "s_1/c_expired",
                json!({
                    "session_id": "s_1",
                    "turn_id": "t_1",
                    "function_call_id": "c_expired",
                    "function_id": "shell::run",
                    "pending_at": 100,
                    "expires_at": 200,
                    "depth": 0,
                }),
            )
            .await;
            state_set(
                &stack.iii,
                PENDING_SCOPE,
                "s_1/c_live",
                json!({
                    "session_id": "s_1",
                    "turn_id": "t_1",
                    "function_call_id": "c_live",
                    "function_id": "shell::run",
                    "pending_at": 100,
                    "expires_at": now_ms() + 60_000,
                    "depth": 0,
                }),
            )
            .await;

            let res = handle(&stack.deps, Value::Null).await.unwrap();
            assert_eq!(res["swept"], json!(1));
            assert!(state_get(&stack.iii, PENDING_SCOPE, "s_1/c_expired")
                .await
                .is_null());
            assert!(!state_get(&stack.iii, PENDING_SCOPE, "s_1/c_live")
                .await
                .is_null());

            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness.len(), 1);
            assert_eq!(harness[0]["details"]["status"], json!("timeout"));
            assert!(wait_for_resolved(&stack).await, "expected pending_resolved");
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_scope_is_a_noop() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let res = handle(&stack.deps, Value::Null).await.unwrap();
            assert_eq!(res["swept"], json!(0));
        })
        .await;
    }

    async fn wait_for_resolved(stack: &crate::testkit::TestStack) -> bool {
        crate::testkit::wait_for(3_000, || !log_snapshot(&stack.resolved).is_empty()).await
    }
}
