//! `approval::sweep` — the cron-bound GC backstop (approval-gate.md §
//! Sweep). For every record past `expires_at`: settle the held call with
//! a timeout denial, delete through the emit gate, fire
//! `pending_resolved { outcome: "timeout" }`. Also collects records
//! orphaned by a crash between resolve and delete — which is why no
//! delete path needs to be transactional.

use serde_json::{json, Value};

use super::Deps;
use crate::error::ApprovalError;
use crate::pending;
use crate::types::{
    now_ms, text_block, PendingApprovalRecord, PendingResolvedEvent, ResolvedOutcome,
};

pub async fn handle(deps: &Deps, _payload: Value) -> Result<Value, ApprovalError> {
    let bus = deps.bus.as_ref();
    let now = now_ms();

    let records = match pending::list_all(bus, deps.cfg.state_timeout_ms).await {
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
        if let Err(e) = bus
            .call(
                "harness::function::resolve",
                resolve_payload,
                Some(deps.cfg.harness_timeout_ms),
            )
            .await
        {
            tracing::warn!(
                session_id = %record.session_id,
                function_call_id = %record.function_call_id,
                error = %e,
                "sweep: harness resolve failed; deleting the expired record anyway"
            );
        }

        match pending::delete_with_gate(
            bus,
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
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::config::WorkerConfig;
    use crate::events::RecordingSink;
    use crate::gate_config::shared_defaults;
    use crate::pending::PENDING_SCOPE;
    use crate::testkit::{FakeBus, MemoryState};

    struct Fixture {
        deps: Arc<Deps>,
        bus: Arc<FakeBus>,
        sink: Arc<RecordingSink>,
        state: MemoryState,
    }

    fn fixture() -> Fixture {
        let bus = Arc::new(FakeBus::new());
        let state = bus.with_memory_state();
        let sink = Arc::new(RecordingSink::new());
        let deps = Arc::new(Deps {
            bus: bus.clone(),
            sink: sink.clone(),
            defaults: shared_defaults(),
            cfg: Arc::new(WorkerConfig::default()),
        });
        Fixture {
            deps,
            bus,
            sink,
            state,
        }
    }

    fn seed(state: &MemoryState, cid: &str, expires_at: i64) {
        state.seed(
            PENDING_SCOPE,
            &format!("s_1/{cid}"),
            json!({
                "session_id": "s_1",
                "turn_id": "t_1",
                "function_call_id": cid,
                "function_id": "shell::run",
                "pending_at": 100,
                "expires_at": expires_at,
            }),
        );
    }

    #[tokio::test]
    async fn sweeps_only_expired_records() {
        let f = fixture();
        f.bus.on_value(
            "harness::function::resolve",
            json!({ "resolved": true, "turn_resumed": true }),
        );
        seed(&f.state, "c_expired", 200); // long past
        seed(&f.state, "c_live", now_ms() + 60_000);

        let res = handle(&f.deps, Value::Null).await.unwrap();
        assert_eq!(res["swept"], json!(1));
        assert!(f.state.peek(PENDING_SCOPE, "s_1/c_expired").is_none());
        assert!(f.state.peek(PENDING_SCOPE, "s_1/c_live").is_some());

        let calls = f.bus.calls_to("harness::function::resolve");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].payload["action"], json!("deliver"));
        assert_eq!(calls[0].payload["is_error"], json!(true));
        assert_eq!(calls[0].payload["details"]["status"], json!("timeout"));

        let events = f.sink.resolved_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, ResolvedOutcome::Timeout);
    }

    #[tokio::test]
    async fn deletes_expired_records_even_when_the_harness_is_absent() {
        let f = fixture();
        // No harness scripted: resolve calls error (standalone deployment).
        seed(&f.state, "c_expired", 200);
        let res = handle(&f.deps, Value::Null).await.unwrap();
        assert_eq!(res["swept"], json!(1));
        assert!(f.state.peek(PENDING_SCOPE, "s_1/c_expired").is_none());
        assert_eq!(f.sink.resolved_events().len(), 1);
    }

    #[tokio::test]
    async fn already_deleted_records_emit_nothing() {
        let f = fixture();
        f.bus
            .on_value("harness::function::resolve", json!({ "resolved": false }));
        seed(&f.state, "c_expired", 200);
        // Simulate a racing path that deleted between list and
        // delete_with_gate: the null write observes no prior value.
        f.bus.on("state::set", |_| {
            Ok(json!({ "old_value": null, "new_value": null }))
        });
        let res = handle(&f.deps, Value::Null).await.unwrap();
        assert_eq!(res["swept"], json!(0));
        assert!(f.sink.resolved_events().is_empty());
    }

    #[tokio::test]
    async fn empty_scope_is_a_noop() {
        let f = fixture();
        let res = handle(&f.deps, Value::Null).await.unwrap();
        assert_eq!(res["swept"], json!(0));
    }
}
