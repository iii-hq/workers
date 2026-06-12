//! `approval::resolve` — apply a human decision to a held call
//! (approval-gate.md § Decision flow). No decision record is persisted —
//! the decision flows straight into `harness::function::resolve`, the
//! transcript carries the durable outcome, and the pending record dies
//! with the resolution.
//!
//! Crash ordering: `harness::function::resolve` FIRST, then delete, then
//! emit — a crash between the first two leaks one record until the sweep
//! collects it; it can never lose a decision.

use serde_json::json;

use super::Deps;
use crate::denial::{render_text, user_deny_envelope};
use crate::error::ApprovalError;
use crate::pending;
use crate::types::{
    now_ms, validate_id, PendingResolvedEvent, ResolveDecision, ResolveRequest, ResolveResponse,
    ResolvedOutcome,
};

pub async fn handle(deps: &Deps, req: ResolveRequest) -> Result<ResolveResponse, ApprovalError> {
    validate_id("session_id", &req.session_id)?;
    validate_id("function_call_id", &req.function_call_id)?;

    let bus = deps.bus.as_ref();
    let Some(record) = pending::get(
        bus,
        &req.session_id,
        &req.function_call_id,
        deps.cfg.state_timeout_ms,
    )
    .await
    .map_err(|e| ApprovalError::StateUnavailable(format!("pending record read failed: {e}")))?
    else {
        // Unknown / already resolved is NOT an error — duplicate
        // decisions race benignly.
        return Ok(ResolveResponse {
            resolved: false,
            turn_resumed: None,
        });
    };

    let payload = match req.decision {
        ResolveDecision::Allow => json!({
            "session_id": req.session_id,
            "turn_id": record.turn_id,
            "function_call_id": req.function_call_id,
            "action": "execute",
        }),
        ResolveDecision::Deny => {
            let envelope = user_deny_envelope(
                &record.function_id,
                req.reason.as_deref(),
                Some(record.arguments_excerpt.clone()),
            );
            json!({
                "session_id": req.session_id,
                "turn_id": record.turn_id,
                "function_call_id": req.function_call_id,
                "action": "deliver",
                "is_error": true,
                "content": render_text(&envelope),
                "details": envelope,
            })
        }
    };

    // The record is kept on failure so the decision stays resolvable
    // (or sweepable) — never delete before the harness acknowledged.
    let reply = bus
        .call(
            "harness::function::resolve",
            payload,
            Some(deps.cfg.harness_timeout_ms),
        )
        .await
        .map_err(|e| {
            ApprovalError::HarnessUnavailable(format!("harness::function::resolve failed: {e}"))
        })?;
    let turn_resumed = reply
        .get("turn_resumed")
        .and_then(serde_json::Value::as_bool);

    match pending::delete_with_gate(
        bus,
        &req.session_id,
        &req.function_call_id,
        deps.cfg.state_timeout_ms,
    )
    .await
    {
        Ok(Some(deleted)) => {
            deps.sink
                .pending_resolved(&PendingResolvedEvent {
                    session_id: deleted.session_id,
                    turn_id: deleted.turn_id,
                    function_call_id: deleted.function_call_id,
                    function_id: deleted.function_id,
                    outcome: match req.decision {
                        ResolveDecision::Allow => ResolvedOutcome::Allow,
                        ResolveDecision::Deny => ResolvedOutcome::Deny,
                    },
                    reason: match req.decision {
                        ResolveDecision::Deny => req.reason.clone(),
                        ResolveDecision::Allow => None,
                    },
                    session_metadata: deleted.session_metadata,
                    resolved_at: now_ms(),
                })
                .await;
        }
        // A concurrent path already deleted (and emitted): exactly-once
        // emission is the gate's contract.
        Ok(None) => {}
        Err(e) => {
            // The decision reached the harness; the orphaned record is
            // sweep food. Never fail the resolve over cleanup.
            tracing::warn!(
                session_id = %req.session_id,
                function_call_id = %req.function_call_id,
                error = %e,
                "pending record delete failed after resolve; sweep will collect it"
            );
        }
    }

    Ok(ResolveResponse {
        resolved: true,
        turn_resumed,
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
    use crate::types::PendingApprovalRecord;

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

    fn seed_record(f: &Fixture) {
        let record = PendingApprovalRecord {
            session_id: "s_1".into(),
            turn_id: "t_9".into(),
            function_call_id: "c_1".into(),
            function_id: "shell::run".into(),
            arguments_excerpt: json!({ "cmd": "ls" }),
            pending_at: 100,
            expires_at: 1_800_100,
            session_title: None,
            session_description: None,
            session_metadata: Some(serde_json::from_value(json!({ "owner": "u_1" })).unwrap()),
            depth: 0,
            assistant_excerpt: None,
        };
        f.state.seed(
            PENDING_SCOPE,
            "s_1/c_1",
            serde_json::to_value(record).unwrap(),
        );
    }

    fn harness_ok(f: &Fixture) {
        f.bus.on_value(
            "harness::function::resolve",
            json!({ "resolved": true, "turn_resumed": true }),
        );
    }

    fn req(decision: ResolveDecision, reason: Option<&str>) -> ResolveRequest {
        ResolveRequest {
            session_id: "s_1".into(),
            function_call_id: "c_1".into(),
            decision,
            reason: reason.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn allow_releases_via_action_execute() {
        let f = fixture();
        seed_record(&f);
        harness_ok(&f);

        let res = handle(&f.deps, req(ResolveDecision::Allow, None))
            .await
            .unwrap();
        assert!(res.resolved);
        assert_eq!(res.turn_resumed, Some(true));

        let calls = f.bus.calls_to("harness::function::resolve");
        assert_eq!(calls.len(), 1);
        let payload = &calls[0].payload;
        assert_eq!(payload["action"], json!("execute"));
        // The turn_id comes from the pending record.
        assert_eq!(payload["turn_id"], json!("t_9"));
        // Execute supplies no result.
        assert!(payload.get("content").is_none());
        assert!(payload.get("is_error").is_none());

        // Record deleted; exactly one resolved event with the metadata.
        assert!(f.state.peek(PENDING_SCOPE, "s_1/c_1").is_none());
        let events = f.sink.resolved_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, ResolvedOutcome::Allow);
        assert_eq!(
            events[0].session_metadata.as_ref().unwrap()["owner"],
            json!("u_1")
        );
    }

    #[tokio::test]
    async fn deny_delivers_an_is_error_denial_envelope() {
        let f = fixture();
        seed_record(&f);
        harness_ok(&f);

        let res = handle(&f.deps, req(ResolveDecision::Deny, Some("too risky")))
            .await
            .unwrap();
        assert!(res.resolved);

        let calls = f.bus.calls_to("harness::function::resolve");
        let payload = &calls[0].payload;
        assert_eq!(payload["action"], json!("deliver"));
        assert_eq!(payload["is_error"], json!(true));
        assert_eq!(payload["content"][0]["type"], json!("text"));
        assert_eq!(payload["content"][0]["text"], json!("too risky"));
        assert_eq!(payload["details"]["denied_by"], json!("user"));
        assert_eq!(payload["details"]["args_excerpt"], json!({ "cmd": "ls" }));

        let events = f.sink.resolved_events();
        assert_eq!(events[0].outcome, ResolvedOutcome::Deny);
        assert_eq!(events[0].reason.as_deref(), Some("too risky"));
    }

    #[tokio::test]
    async fn unknown_call_returns_resolved_false_and_emits_nothing() {
        let f = fixture();
        harness_ok(&f);
        let res = handle(&f.deps, req(ResolveDecision::Allow, None))
            .await
            .unwrap();
        assert!(!res.resolved);
        assert!(f.bus.calls_to("harness::function::resolve").is_empty());
        assert!(f.sink.resolved_events().is_empty());
    }

    #[tokio::test]
    async fn duplicate_resolve_races_benignly() {
        let f = fixture();
        seed_record(&f);
        harness_ok(&f);
        let first = handle(&f.deps, req(ResolveDecision::Allow, None))
            .await
            .unwrap();
        assert!(first.resolved);
        let second = handle(&f.deps, req(ResolveDecision::Allow, None))
            .await
            .unwrap();
        assert!(!second.resolved);
        assert_eq!(f.sink.resolved_events().len(), 1);
    }

    #[tokio::test]
    async fn harness_failure_keeps_the_record() {
        let f = fixture();
        seed_record(&f);
        f.bus
            .on_error("harness::function::resolve", "connection refused");
        let err = handle(&f.deps, req(ResolveDecision::Allow, None))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "approval/harness_unavailable");
        // Record intact, no event: the decision can be retried.
        assert!(f.state.peek(PENDING_SCOPE, "s_1/c_1").is_some());
        assert!(f.sink.resolved_events().is_empty());
    }

    #[tokio::test]
    async fn ordering_is_resolve_then_delete() {
        let f = fixture();
        seed_record(&f);
        harness_ok(&f);
        handle(&f.deps, req(ResolveDecision::Allow, None))
            .await
            .unwrap();
        let calls = f.bus.calls();
        let resolve_pos = calls
            .iter()
            .position(|c| c.function_id == "harness::function::resolve")
            .unwrap();
        let delete_pos = calls
            .iter()
            .position(|c| c.function_id == "state::set" && c.payload["value"].is_null())
            .unwrap();
        assert!(resolve_pos < delete_pos);
    }

    #[tokio::test]
    async fn invalid_ids_are_rejected() {
        let f = fixture();
        let err = handle(
            &f.deps,
            ResolveRequest {
                session_id: "s/1".into(),
                function_call_id: "c_1".into(),
                decision: ResolveDecision::Allow,
                reason: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), "approval/invalid_payload");
    }
}
