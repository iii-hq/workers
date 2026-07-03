//! `approval::resolve` — apply a human decision to a held call
//! (approval-gate.md § Decision flow). No decision record is persisted —
//! the decision flows straight into `harness::function::resolve`, the
//! transcript carries the durable outcome, and the pending record dies
//! with the resolution.
//!
//! Crash ordering: `harness::function::resolve` FIRST, then delete, then
//! emit — a crash between the first two leaks one record until turn/session
//! cleanup; it can never lose a decision.
//!
//! `folder_access` records (held by `approval::grant-watch`) additionally
//! apply `req.grant_scope` on allow (spec-pr3-approval-gate.md § Resolve
//! orchestration): `once` rides the release as a one-shot `extra_roots`,
//! `session`/`always` install a durable `harness::workspace::grant` first
//! (falling back to once-style `extra_roots` on failure so the user's click
//! still works), and `always` additionally best-effort persists the dir into
//! the `shell` deployment configuration. `grant_scope` on a plain `function`
//! record — or on a `deny` decision — is ignored (logged).

use serde_json::{json, Value};

use super::Deps;
use crate::denial::{render_text, user_deny_envelope};
use crate::error::ApprovalError;
use crate::grant_state;
use crate::harness;
use crate::pending;
use crate::shell_config;
use crate::types::{
    now_ms, validate_id, GrantRequest, GrantScope, PendingApprovalRecord, PendingKind,
    PendingResolvedEvent, ResolveDecision, ResolveRequest, ResolveResponse, ResolvedOutcome,
};

pub async fn handle(deps: &Deps, req: ResolveRequest) -> Result<ResolveResponse, ApprovalError> {
    validate_id("session_id", &req.session_id)?;
    validate_id("function_call_id", &req.function_call_id)?;

    let iii = deps.iii.as_ref();
    let Some(record) = pending::get(iii, &req.session_id, &req.function_call_id)
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

    if req.grant_scope.is_some() && record.kind != PendingKind::FolderAccess {
        tracing::warn!(
            session_id = %req.session_id,
            function_call_id = %req.function_call_id,
            function_id = %record.function_id,
            "grant_scope ignored: pending record is not folder_access"
        );
    }

    let payload = build_payload(deps, &req, &record).await;

    // The record is kept on failure so the decision stays resolvable —
    // never delete before the harness acknowledged.
    let reply = harness::function_resolve(iii, payload).await.map_err(|e| {
        ApprovalError::HarnessUnavailable(format!("harness::function::resolve failed: {e}"))
    })?;
    let turn_resumed = reply
        .get("turn_resumed")
        .and_then(serde_json::Value::as_bool);

    match pending::delete_with_gate(iii, &req.session_id, &req.function_call_id).await {
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
            // The decision reached the harness; retry or turn/session cleanup
            // will collect the orphaned record. Never fail resolve over cleanup.
            tracing::warn!(
                session_id = %req.session_id,
                function_call_id = %req.function_call_id,
                error = %e,
                "pending record delete failed after resolve; retry or purge will collect it"
            );
        }
    }

    Ok(ResolveResponse {
        resolved: true,
        turn_resumed,
    })
}

/// Build the `harness::function::resolve` payload for `req`/`record`.
/// `function` records (and `folder_access` records under `deny`) build the
/// same payload as before this feature existed; `folder_access` + `allow`
/// additionally applies `grant_scope`.
async fn build_payload(deps: &Deps, req: &ResolveRequest, record: &PendingApprovalRecord) -> Value {
    match req.decision {
        ResolveDecision::Allow if record.kind == PendingKind::FolderAccess => {
            build_grant_allow_payload(deps, req, record).await
        }
        ResolveDecision::Allow => execute_payload(req, &record.turn_id, None),
        ResolveDecision::Deny if record.kind == PendingKind::FolderAccess => {
            build_grant_deny_payload(deps, req, record).await
        }
        ResolveDecision::Deny => deliver_denial_payload(req, record, req.reason.as_deref()),
    }
}

fn execute_payload(req: &ResolveRequest, turn_id: &str, extra_roots: Option<&[String]>) -> Value {
    let mut payload = json!({
        "session_id": req.session_id,
        "turn_id": turn_id,
        "function_call_id": req.function_call_id,
        "action": "execute",
    });
    if let Some(dirs) = extra_roots {
        payload["extra_roots"] = json!(dirs);
    }
    payload
}

fn deliver_denial_payload(
    req: &ResolveRequest,
    record: &PendingApprovalRecord,
    reason: Option<&str>,
) -> Value {
    let envelope = user_deny_envelope(
        &record.function_id,
        reason,
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

/// `allow` on a `folder_access` record: apply `grant_scope` (default
/// `once`) — spec-pr3-approval-gate.md § Resolve orchestration.
async fn build_grant_allow_payload(
    deps: &Deps,
    req: &ResolveRequest,
    record: &PendingApprovalRecord,
) -> Value {
    let Some(grant) = &record.grant_request else {
        // Defensive: a folder_access record without a grant_request should
        // never happen (grant-watch always sets it), but a missing dir
        // must not crash resolve — behave like a plain execute.
        tracing::warn!(
            function_call_id = %req.function_call_id,
            "folder_access record missing grant_request; releasing without extra_roots"
        );
        return execute_payload(req, &record.turn_id, None);
    };

    let iii = deps.iii.as_ref();
    match req.grant_scope.unwrap_or(GrantScope::Once) {
        GrantScope::Once => once_payload(req, &record.turn_id, grant),
        GrantScope::Session => {
            match apply_workspace_grant(iii, &req.session_id, &grant.dir).await {
                Ok(()) => execute_payload(req, &record.turn_id, None),
                Err(()) => once_payload(req, &record.turn_id, grant),
            }
        }
        GrantScope::Always => {
            let grant_applied = apply_workspace_grant(iii, &req.session_id, &grant.dir).await;
            persist_host_root_best_effort(iii, &grant.dir).await;
            match grant_applied {
                Ok(()) => execute_payload(req, &record.turn_id, None),
                Err(()) => once_payload(req, &record.turn_id, grant),
            }
        }
    }
}

fn once_payload(req: &ResolveRequest, turn_id: &str, grant: &GrantRequest) -> Value {
    execute_payload(req, turn_id, Some(std::slice::from_ref(&grant.dir)))
}

/// `harness::workspace::grant` — its failure is never fatal to the
/// release, only to durability: the caller falls back to once-style
/// `extra_roots` so the user's click still works.
async fn apply_workspace_grant(
    iii: &iii_sdk::IIIClient,
    session_id: &str,
    dir: &str,
) -> Result<(), ()> {
    match harness::workspace_grant(iii, session_id, dir).await {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::warn!(
                session_id,
                dir,
                error = %e,
                "harness::workspace::grant failed; falling back to once-style extra_roots for this release"
            );
            Err(())
        }
    }
}

async fn persist_host_root_best_effort(iii: &iii_sdk::IIIClient, dir: &str) {
    if let Err(e) = shell_config::add_host_root(iii, dir).await {
        tracing::warn!(
            dir,
            error = %e,
            "best-effort shell fs.host_roots persist failed; the session-scoped grant (if any) still applies"
        );
    }
}

/// `deny` on a `folder_access` record: remember the denial for the rest of
/// the session BEFORE delivering, and default the reason to mention folder
/// access specifically.
async fn build_grant_deny_payload(
    deps: &Deps,
    req: &ResolveRequest,
    record: &PendingApprovalRecord,
) -> Value {
    let dir = record.grant_request.as_ref().map(|g| g.dir.as_str());
    if let Some(dir) = dir {
        if let Err(e) = grant_state::add_denied(deps.iii.as_ref(), &req.session_id, dir).await {
            tracing::warn!(
                session_id = %req.session_id,
                dir,
                error = %e,
                "denied-memory write failed; grant-watch may re-ask for this dir again"
            );
        }
    }

    let default_reason = dir.map(|d| format!("user denied folder access to {d}"));
    let reason = req
        .reason
        .as_deref()
        .filter(|r| !r.is_empty())
        .or(default_reason.as_deref());
    deliver_denial_payload(req, record, reason)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::pending::PENDING_SCOPE;
    use crate::testkit::{
        log_push, log_snapshot, state_get, state_set, with_stack, BootOpts, TestStack,
    };

    async fn seed_record(iii: &iii_sdk::IIIClient) {
        let record = PendingApprovalRecord {
            session_id: "s_1".into(),
            turn_id: "t_9".into(),
            function_call_id: "c_1".into(),
            function_id: "shell::run".into(),
            arguments_excerpt: json!({ "cmd": "ls" }),
            pending_at: 100,
            session_title: None,
            session_description: None,
            session_metadata: Some(serde_json::from_value(json!({ "owner": "u_1" })).unwrap()),
            depth: 0,
            assistant_excerpt: None,
            kind: PendingKind::Function,
            grant_request: None,
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
            grant_scope: None,
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
                    grant_scope: None,
                },
            )
            .await
            .unwrap_err();
            assert_eq!(err.code(), "approval/invalid_payload");
        })
        .await;
    }

    // -----------------------------------------------------------------
    // folder_access + grant_scope
    // -----------------------------------------------------------------

    async fn seed_folder_access_record(iii: &iii_sdk::IIIClient, dir: &str) {
        let record = PendingApprovalRecord {
            session_id: "s_1".into(),
            turn_id: "t_9".into(),
            function_call_id: "c_1".into(),
            function_id: "shell::fs::read".into(),
            arguments_excerpt: json!({ "path": dir }),
            pending_at: 100,
            session_title: None,
            session_description: None,
            session_metadata: None,
            depth: 0,
            assistant_excerpt: None,
            kind: PendingKind::FolderAccess,
            grant_request: Some(GrantRequest {
                dir: dir.to_string(),
                offending_path: format!("{dir}/x"),
                error_code: "S215".to_string(),
            }),
        };
        state_set(
            iii,
            PENDING_SCOPE,
            "s_1/c_1",
            serde_json::to_value(record).unwrap(),
        )
        .await;
    }

    fn grant_req(scope: Option<GrantScope>) -> ResolveRequest {
        ResolveRequest {
            session_id: "s_1".into(),
            function_call_id: "c_1".into(),
            decision: ResolveDecision::Allow,
            reason: None,
            grant_scope: scope,
        }
    }

    /// Fake `harness::workspace::grant`, logging to `stack.harness_calls`
    /// tagged so a test can tell it apart from `harness::function::resolve`
    /// deliveries.
    fn register_workspace_grant_fake(stack: &TestStack, fail: bool) {
        let iii = stack.iii.clone();
        let log = stack.harness_calls.clone();
        iii.register_function(
            "harness::workspace::grant",
            iii_sdk::RegisterFunction::new_async(move |req: Value| {
                let log = log.clone();
                async move {
                    if fail {
                        return Err(iii_sdk::errors::Error::Handler("boom".into()));
                    }
                    let mut tagged = req.clone();
                    tagged["__fn"] = json!("harness::workspace::grant");
                    log_push(&log, tagged);
                    let root = req.get("root").cloned().unwrap_or(Value::Null);
                    Ok::<_, iii_sdk::errors::Error>(json!({
                        "session_id": req.get("session_id"),
                        "roots": [root],
                    }))
                }
            }),
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn once_scope_carries_extra_roots_and_does_not_call_workspace_grant() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_folder_access_record(&stack.iii, "/a/b").await;
            register_workspace_grant_fake(&stack, false);

            let res = handle(&stack.deps, grant_req(Some(GrantScope::Once)))
                .await
                .unwrap();
            assert!(res.resolved);

            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness.len(), 1, "only function::resolve, no grant call");
            assert_eq!(harness[0]["action"], json!("execute"));
            assert_eq!(harness[0]["extra_roots"], json!(["/a/b"]));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn default_grant_scope_behaves_like_once() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_folder_access_record(&stack.iii, "/a/b").await;
            register_workspace_grant_fake(&stack, false);

            handle(&stack.deps, grant_req(None)).await.unwrap();

            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness[0]["extra_roots"], json!(["/a/b"]));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn session_scope_grants_before_execute_and_omits_extra_roots() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_folder_access_record(&stack.iii, "/a/b").await;
            register_workspace_grant_fake(&stack, false);

            handle(&stack.deps, grant_req(Some(GrantScope::Session)))
                .await
                .unwrap();

            let calls = log_snapshot(&stack.harness_calls);
            let grant_call = calls
                .iter()
                .find(|c| c["__fn"] == json!("harness::workspace::grant"))
                .expect("workspace::grant was called");
            assert_eq!(grant_call["root"], json!("/a/b"));
            assert_eq!(grant_call["session_id"], json!("s_1"));

            let execute_call = calls
                .iter()
                .find(|c| c["action"] == json!("execute"))
                .expect("execute call");
            assert!(execute_call.get("extra_roots").is_none());
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn session_scope_falls_back_to_extra_roots_when_grant_fails() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_folder_access_record(&stack.iii, "/a/b").await;
            register_workspace_grant_fake(&stack, true);

            let res = handle(&stack.deps, grant_req(Some(GrantScope::Session)))
                .await
                .unwrap();
            assert!(
                res.resolved,
                "the release still happens despite the grant failure"
            );

            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness.len(), 1);
            assert_eq!(harness[0]["action"], json!("execute"));
            assert_eq!(harness[0]["extra_roots"], json!(["/a/b"]));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn always_scope_grants_and_persists_configuration_best_effort() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_folder_access_record(&stack.iii, "/a/b").await;
            register_workspace_grant_fake(&stack, false);
            crate::testkit::call(
                &stack.iii,
                "configuration::register",
                json!({
                    "id": "shell",
                    "name": "shell",
                    "description": "test seed",
                    "schema": { "type": "object" },
                    "initial_value": {},
                }),
            )
            .await
            .expect("register shell configuration");

            handle(&stack.deps, grant_req(Some(GrantScope::Always)))
                .await
                .unwrap();

            let calls = log_snapshot(&stack.harness_calls);
            assert!(calls
                .iter()
                .any(|c| c["__fn"] == json!("harness::workspace::grant")));
            let execute_call = calls
                .iter()
                .find(|c| c["action"] == json!("execute"))
                .expect("execute call");
            assert!(execute_call.get("extra_roots").is_none());

            let shell_cfg =
                crate::testkit::call(&stack.iii, "configuration::get", json!({ "id": "shell" }))
                    .await
                    .unwrap();
            assert_eq!(
                shell_cfg["value"]["fs"]["host_roots"],
                json!(["/a/b"]),
                "always scope persists the dir into shell's fs.host_roots"
            );
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn always_scope_still_executes_when_configuration_persist_is_unavailable() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_folder_access_record(&stack.iii, "/a/b").await;
            register_workspace_grant_fake(&stack, false);
            // No `shell` configuration id registered -> configuration::get
            // fails -> add_host_root fails -> logged, never blocks execute.

            let res = handle(&stack.deps, grant_req(Some(GrantScope::Always)))
                .await
                .unwrap();
            assert!(res.resolved);

            let calls = log_snapshot(&stack.harness_calls);
            let execute_call = calls
                .iter()
                .find(|c| c["action"] == json!("execute"))
                .expect("execute call");
            assert!(execute_call.get("extra_roots").is_none());
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deny_records_the_dir_and_defaults_a_folder_access_reason() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_folder_access_record(&stack.iii, "/a/b").await;

            let req = ResolveRequest {
                session_id: "s_1".into(),
                function_call_id: "c_1".into(),
                decision: ResolveDecision::Deny,
                reason: None,
                grant_scope: None,
            };
            handle(&stack.deps, req).await.unwrap();

            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness[0]["action"], json!("deliver"));
            assert_eq!(
                harness[0]["content"][0]["text"],
                json!("user denied folder access to /a/b")
            );
            assert_eq!(harness[0]["details"]["denied_by"], json!("user"));

            let denied = state_get(&stack.iii, grant_state::DENIED_SCOPE, "s_1").await;
            assert_eq!(denied, json!(["/a/b"]));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_scope_on_a_plain_function_record_is_ignored() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_record(&stack.iii).await;
            let req = ResolveRequest {
                session_id: "s_1".into(),
                function_call_id: "c_1".into(),
                decision: ResolveDecision::Allow,
                reason: None,
                grant_scope: Some(GrantScope::Always),
            };
            handle(&stack.deps, req).await.unwrap();

            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness.len(), 1);
            assert_eq!(harness[0]["action"], json!("execute"));
            assert!(harness[0].get("extra_roots").is_none());
        })
        .await;
    }
}
