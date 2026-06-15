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
use crate::harness;
use crate::pending;
use crate::types::{
    now_ms, validate_id, PendingResolvedEvent, ResolveDecision, ResolveRequest, ResolveResponse,
    ResolvedOutcome,
};

pub async fn handle(deps: &Deps, req: ResolveRequest) -> Result<ResolveResponse, ApprovalError> {
    validate_id("session_id", &req.session_id)?;
    validate_id("function_call_id", &req.function_call_id)?;

    let iii = deps.iii.as_ref();
    let Some(record) = pending::get(
        iii,
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
    let reply = harness::function_resolve(iii, payload, Some(deps.cfg.harness_timeout_ms))
        .await
        .map_err(|e| {
            ApprovalError::HarnessUnavailable(format!("harness::function::resolve failed: {e}"))
        })?;
    let turn_resumed = reply
        .get("turn_resumed")
        .and_then(serde_json::Value::as_bool);

    match pending::delete_with_gate(
        iii,
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
    use serde_json::json;

    use super::*;
    use crate::pending::PENDING_SCOPE;
    use crate::testkit::{log_snapshot, state_get, state_set, with_stack, BootOpts};
    use crate::types::PendingApprovalRecord;

    async fn seed_record(iii: &iii_sdk::III) {
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
        state_set(
            iii,
            PENDING_SCOPE,
            "s_1/c_1",
            serde_json::to_value(record).unwrap(),
        )
        .await;
    }

    fn req(decision: ResolveDecision, reason: Option<&str>) -> ResolveRequest {
        ResolveRequest {
            session_id: "s_1".into(),
            function_call_id: "c_1".into(),
            decision,
            reason: reason.map(str::to_string),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn allow_releases_via_action_execute() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_record(&stack.iii).await;
            let res = handle(&stack.deps, req(ResolveDecision::Allow, None))
                .await
                .unwrap();
            assert!(res.resolved);
            assert_eq!(res.turn_resumed, Some(true));

            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness.len(), 1);
            assert_eq!(harness[0]["action"], json!("execute"));
            assert_eq!(harness[0]["turn_id"], json!("t_9"));
            assert!(state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1")
                .await
                .is_null());
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deny_delivers_an_is_error_denial_envelope() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_record(&stack.iii).await;
            handle(&stack.deps, req(ResolveDecision::Deny, Some("too risky")))
                .await
                .unwrap();
            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness[0]["action"], json!("deliver"));
            assert_eq!(harness[0]["is_error"], json!(true));
            assert_eq!(harness[0]["content"][0]["text"], json!("too risky"));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_call_returns_resolved_false() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let res = handle(&stack.deps, req(ResolveDecision::Allow, None))
                .await
                .unwrap();
            assert!(!res.resolved);
            assert!(log_snapshot(&stack.harness_calls).is_empty());
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_ids_are_rejected() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let err = handle(
                &stack.deps,
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
        })
        .await;
    }
}
