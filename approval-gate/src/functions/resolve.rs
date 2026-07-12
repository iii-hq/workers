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
//! `filesystem_access` records (held by `approval::filesystem-access-watch`) additionally
//! apply `req.access_duration` on allow (spec-pr3-approval-gate.md § Resolve
//! orchestration): `once` rides the release as a one-shot `fs_scope.grants`,
//! `session`/`always` install a durable `harness::filesystem::grant` first
//! (falling back to once-style `fs_scope.grants` on failure so the user's click
//! still works), and `always` additionally best-effort persists the root into
//! the `shell` deployment configuration. `access_duration` on a plain `function`
//! record — or on a `deny` decision — is ignored (logged).

use serde_json::{json, Value};

use super::Deps;
use crate::denial::{render_text, user_deny_envelope};
use crate::error::ApprovalError;
use crate::filesystem_access_state;
use crate::harness;
use crate::pending;
use crate::shell_config;
use crate::types::{
    now_ms, validate_id, AccessDuration, AccessRequest, PendingApprovalRecord, PendingKind,
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

    if req.access_duration.is_some() && record.kind != PendingKind::FilesystemAccess {
        tracing::warn!(
            session_id = %req.session_id,
            function_call_id = %req.function_call_id,
            function_id = %record.function_id,
            "access_duration ignored: pending record is not filesystem_access"
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

    // Executing an approved call can synchronously hit another hold, most
    // notably the filesystem-access post-trigger hook. That follow-up replaces
    // the original record under the same session/call key before harness
    // returns. Delete only when the key still contains the record we resolved;
    // otherwise preserve the replacement. A verification failure is also
    // cleanup-only: the function has already executed, so returning an error
    // here could encourage a duplicate retry.
    let next_pending = match pending::get(iii, &req.session_id, &req.function_call_id).await {
        Ok(current) => current.filter(|current| current != &record),
        Err(e) => {
            tracing::warn!(
                session_id = %req.session_id,
                function_call_id = %req.function_call_id,
                error = %e,
                "pending record verification failed after resolve; preserving it for retry or purge"
            );
            return Ok(ResolveResponse {
                resolved: true,
                turn_resumed,
            });
        }
    };

    if let Some(next) = &next_pending {
        tracing::info!(
            session_id = %req.session_id,
            function_call_id = %req.function_call_id,
            next_kind = ?next.kind,
            next_pending_at = next.pending_at,
            "approved call transitioned to a follow-up approval"
        );
    } else {
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
    }

    Ok(ResolveResponse {
        resolved: true,
        turn_resumed,
    })
}

/// Build the `harness::function::resolve` payload for `req`/`record`.
/// `function` records (and `filesystem_access` records under `deny`) build the
/// same payload as before this feature existed; `filesystem_access` + `allow`
/// additionally applies `access_duration`.
async fn build_payload(deps: &Deps, req: &ResolveRequest, record: &PendingApprovalRecord) -> Value {
    match req.decision {
        ResolveDecision::Allow if record.kind == PendingKind::FilesystemAccess => {
            build_grant_allow_payload(deps, req, record).await
        }
        ResolveDecision::Allow => execute_payload(req, &record.turn_id, None),
        ResolveDecision::Deny if record.kind == PendingKind::FilesystemAccess => {
            build_grant_deny_payload(deps, req, record).await
        }
        ResolveDecision::Deny => deliver_denial_payload(req, record, req.reason.as_deref()),
    }
}

fn execute_payload(req: &ResolveRequest, turn_id: &str, grants: Option<&[String]>) -> Value {
    let mut payload = json!({
        "session_id": req.session_id,
        "turn_id": turn_id,
        "function_call_id": req.function_call_id,
        "action": "execute",
    });
    if let Some(dirs) = grants {
        payload["fs_scope"] = json!({ "grants": dirs });
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

/// `allow` on a `filesystem_access` record: apply `access_duration` (default
/// `once`) — spec-pr3-approval-gate.md § Resolve orchestration.
async fn build_grant_allow_payload(
    deps: &Deps,
    req: &ResolveRequest,
    record: &PendingApprovalRecord,
) -> Value {
    let Some(grant) = &record.access_request else {
        // Defensive: a filesystem_access record without a access_request should
        // never happen (filesystem-access-watch always sets it), but a missing access request root
        // must not crash resolve — behave like a plain execute.
        tracing::warn!(
            function_call_id = %req.function_call_id,
            "filesystem_access record missing access_request; releasing without fs_scope.grants"
        );
        return execute_payload(req, &record.turn_id, None);
    };

    let iii = deps.iii.as_ref();
    match req.access_duration.unwrap_or(AccessDuration::Once) {
        AccessDuration::Once => once_payload(req, &record.turn_id, grant),
        AccessDuration::Session => {
            match apply_filesystem_grant(iii, &req.session_id, &grant.requested_root).await {
                Ok(()) => execute_payload(req, &record.turn_id, None),
                Err(()) => once_payload(req, &record.turn_id, grant),
            }
        }
        AccessDuration::Always => {
            let grant_applied =
                apply_filesystem_grant(iii, &req.session_id, &grant.requested_root).await;
            persist_host_root_best_effort(iii, &grant.requested_root).await;
            match grant_applied {
                Ok(()) => execute_payload(req, &record.turn_id, None),
                Err(()) => once_payload(req, &record.turn_id, grant),
            }
        }
    }
}

fn once_payload(req: &ResolveRequest, turn_id: &str, grant: &AccessRequest) -> Value {
    execute_payload(
        req,
        turn_id,
        Some(std::slice::from_ref(&grant.requested_root)),
    )
}

/// `harness::filesystem::grant` — its failure is never fatal to the
/// release, only to durability: the caller falls back to once-style
/// `fs_scope.grants` so the user's click still works.
async fn apply_filesystem_grant(
    iii: &iii_sdk::IIIClient,
    session_id: &str,
    requested_root: &str,
) -> Result<(), ()> {
    match harness::filesystem_grant(iii, session_id, requested_root).await {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::warn!(
                session_id,
                requested_root,
                error = %e,
                "harness::filesystem::grant failed; falling back to once-style fs_scope.grants for this release"
            );
            Err(())
        }
    }
}

async fn persist_host_root_best_effort(iii: &iii_sdk::IIIClient, requested_root: &str) {
    if let Err(e) = shell_config::add_host_root(iii, requested_root).await {
        tracing::warn!(
            requested_root,
            error = %e,
            "best-effort shell fs.host_roots persist failed; the session-scoped grant (if any) still applies"
        );
    }
}

/// `deny` on a `filesystem_access` record: remember the denial for the rest of
/// the session BEFORE delivering, and default the reason to mention filesystem
/// access specifically.
async fn build_grant_deny_payload(
    deps: &Deps,
    req: &ResolveRequest,
    record: &PendingApprovalRecord,
) -> Value {
    let requested_root = record
        .access_request
        .as_ref()
        .map(|g| g.requested_root.as_str());
    if let Some(requested_root) = requested_root {
        if let Err(e) =
            filesystem_access_state::add_denied(deps.iii.as_ref(), &req.session_id, requested_root)
                .await
        {
            tracing::warn!(
                session_id = %req.session_id,
                requested_root,
                error = %e,
                "denied-memory write failed; filesystem-access-watch may re-ask for this root again"
            );
        }
    }

    let default_reason =
        requested_root.map(|root| format!("user denied filesystem access to {root}"));
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
            access_request: None,
        };
        state_set(
            iii,
            PENDING_SCOPE,
            "s_1/c_1",
            serde_json::to_value(record).unwrap(),
        )
        .await;
    }

    fn filesystem_replacement() -> PendingApprovalRecord {
        PendingApprovalRecord {
            session_id: "s_1".into(),
            turn_id: "t_9".into(),
            function_call_id: "c_1".into(),
            function_id: "shell::run".into(),
            arguments_excerpt: json!({ "cmd": "ls" }),
            pending_at: 200,
            session_title: None,
            session_description: None,
            session_metadata: Some(serde_json::from_value(json!({ "owner": "u_1" })).unwrap()),
            depth: 0,
            assistant_excerpt: None,
            kind: PendingKind::FilesystemAccess,
            access_request: Some(AccessRequest {
                requested_root: "/private/tmp".into(),
                attempted_path: "/private/tmp".into(),
                error_code: "S215".into(),
            }),
        }
    }

    fn req(decision: ResolveDecision, reason: Option<&str>) -> ResolveRequest {
        ResolveRequest {
            session_id: "s_1".into(),
            function_call_id: "c_1".into(),
            decision,
            reason: reason.map(str::to_string),
            access_duration: None,
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
    async fn allow_preserves_a_followup_approval_created_during_execution() {
        let replacement = filesystem_replacement();
        let replacement_value = serde_json::to_value(&replacement).unwrap();
        with_stack(
            BootOpts::needs_approval().replacing_pending_on_resolve(replacement_value.clone()),
            |stack| async move {
                seed_record(&stack.iii).await;

                let res = handle(&stack.deps, req(ResolveDecision::Allow, None))
                    .await
                    .unwrap();

                assert!(res.resolved);
                assert_eq!(res.turn_resumed, Some(false));
                assert_eq!(
                    state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1").await,
                    replacement_value,
                    "the follow-up approval must remain actionable"
                );
                assert!(
                    log_snapshot(&stack.resolved).is_empty(),
                    "the replacement must not be cleared as the old approval"
                );
            },
        )
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
                    access_duration: None,
                },
            )
            .await
            .unwrap_err();
            assert_eq!(err.code(), "approval/invalid_payload");
        })
        .await;
    }

    // -----------------------------------------------------------------
    // filesystem_access + access_duration
    // -----------------------------------------------------------------

    async fn seed_filesystem_access_record(iii: &iii_sdk::IIIClient, requested_root: &str) {
        let record = PendingApprovalRecord {
            session_id: "s_1".into(),
            turn_id: "t_9".into(),
            function_call_id: "c_1".into(),
            function_id: "shell::fs::read".into(),
            arguments_excerpt: json!({ "path": requested_root }),
            pending_at: 100,
            session_title: None,
            session_description: None,
            session_metadata: None,
            depth: 0,
            assistant_excerpt: None,
            kind: PendingKind::FilesystemAccess,
            access_request: Some(AccessRequest {
                requested_root: requested_root.to_string(),
                attempted_path: format!("{requested_root}/x"),
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

    fn grant_req(scope: Option<AccessDuration>) -> ResolveRequest {
        ResolveRequest {
            session_id: "s_1".into(),
            function_call_id: "c_1".into(),
            decision: ResolveDecision::Allow,
            reason: None,
            access_duration: scope,
        }
    }

    /// Fake `harness::filesystem::grant`, logging to `stack.harness_calls`
    /// tagged so a test can tell it apart from `harness::function::resolve`
    /// deliveries.
    fn register_filesystem_grant_fake(stack: &TestStack, fail: bool) {
        let iii = stack.iii.clone();
        let log = stack.harness_calls.clone();
        iii.register_function(
            "harness::filesystem::grant",
            iii_sdk::RegisterFunction::new_async(move |req: Value| {
                let log = log.clone();
                async move {
                    if fail {
                        return Err(iii_sdk::errors::Error::Handler("boom".into()));
                    }
                    let mut tagged = req.clone();
                    tagged["__fn"] = json!("harness::filesystem::grant");
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
    async fn once_scope_carries_fs_scope_grants_and_does_not_call_filesystem_grant() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_filesystem_access_record(&stack.iii, "/a/b").await;
            register_filesystem_grant_fake(&stack, false);

            let res = handle(&stack.deps, grant_req(Some(AccessDuration::Once)))
                .await
                .unwrap();
            assert!(res.resolved);

            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness.len(), 1, "only function::resolve, no grant call");
            assert_eq!(harness[0]["action"], json!("execute"));
            assert_eq!(harness[0]["fs_scope"]["grants"], json!(["/a/b"]));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn default_access_duration_behaves_like_once() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_filesystem_access_record(&stack.iii, "/a/b").await;
            register_filesystem_grant_fake(&stack, false);

            handle(&stack.deps, grant_req(None)).await.unwrap();

            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness[0]["fs_scope"]["grants"], json!(["/a/b"]));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn session_scope_grants_before_execute_and_omits_fs_scope_grants() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_filesystem_access_record(&stack.iii, "/a/b").await;
            register_filesystem_grant_fake(&stack, false);

            handle(&stack.deps, grant_req(Some(AccessDuration::Session)))
                .await
                .unwrap();

            let calls = log_snapshot(&stack.harness_calls);
            let grant_call = calls
                .iter()
                .find(|c| c["__fn"] == json!("harness::filesystem::grant"))
                .expect("filesystem::grant was called");
            assert_eq!(grant_call["root"], json!("/a/b"));
            assert_eq!(grant_call["session_id"], json!("s_1"));

            let execute_call = calls
                .iter()
                .find(|c| c["action"] == json!("execute"))
                .expect("execute call");
            assert!(execute_call.get("fs_scope").is_none());
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn session_scope_falls_back_to_fs_scope_grants_when_grant_fails() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_filesystem_access_record(&stack.iii, "/a/b").await;
            register_filesystem_grant_fake(&stack, true);

            let res = handle(&stack.deps, grant_req(Some(AccessDuration::Session)))
                .await
                .unwrap();
            assert!(
                res.resolved,
                "the release still happens despite the grant failure"
            );

            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness.len(), 1);
            assert_eq!(harness[0]["action"], json!("execute"));
            assert_eq!(harness[0]["fs_scope"]["grants"], json!(["/a/b"]));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn always_scope_grants_and_persists_configuration_best_effort() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_filesystem_access_record(&stack.iii, "/a/b").await;
            register_filesystem_grant_fake(&stack, false);
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

            handle(&stack.deps, grant_req(Some(AccessDuration::Always)))
                .await
                .unwrap();

            let calls = log_snapshot(&stack.harness_calls);
            assert!(calls
                .iter()
                .any(|c| c["__fn"] == json!("harness::filesystem::grant")));
            let execute_call = calls
                .iter()
                .find(|c| c["action"] == json!("execute"))
                .expect("execute call");
            assert!(execute_call.get("fs_scope").is_none());

            let shell_cfg =
                crate::testkit::call(&stack.iii, "configuration::get", json!({ "id": "shell" }))
                    .await
                    .unwrap();
            assert_eq!(
                shell_cfg["value"]["fs"]["host_roots"],
                json!(["/a/b"]),
                "always scope persists the root into shell's fs.host_roots"
            );
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn always_scope_still_executes_when_configuration_persist_is_unavailable() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_filesystem_access_record(&stack.iii, "/a/b").await;
            register_filesystem_grant_fake(&stack, false);
            // No `shell` configuration id registered -> configuration::get
            // fails -> add_host_root fails -> logged, never blocks execute.

            let res = handle(&stack.deps, grant_req(Some(AccessDuration::Always)))
                .await
                .unwrap();
            assert!(res.resolved);

            let calls = log_snapshot(&stack.harness_calls);
            let execute_call = calls
                .iter()
                .find(|c| c["action"] == json!("execute"))
                .expect("execute call");
            assert!(execute_call.get("fs_scope").is_none());
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deny_records_the_dir_and_defaults_a_filesystem_access_reason() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_filesystem_access_record(&stack.iii, "/a/b").await;

            let req = ResolveRequest {
                session_id: "s_1".into(),
                function_call_id: "c_1".into(),
                decision: ResolveDecision::Deny,
                reason: None,
                access_duration: None,
            };
            handle(&stack.deps, req).await.unwrap();

            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness[0]["action"], json!("deliver"));
            assert_eq!(
                harness[0]["content"][0]["text"],
                json!("user denied filesystem access to /a/b")
            );
            assert_eq!(harness[0]["details"]["denied_by"], json!("user"));

            let denied = state_get(&stack.iii, filesystem_access_state::DENIED_SCOPE, "s_1").await;
            assert_eq!(denied, json!(["/a/b"]));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn access_duration_on_a_plain_function_record_is_ignored() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_record(&stack.iii).await;
            let req = ResolveRequest {
                session_id: "s_1".into(),
                function_call_id: "c_1".into(),
                decision: ResolveDecision::Allow,
                reason: None,
                access_duration: Some(AccessDuration::Always),
            };
            handle(&stack.deps, req).await.unwrap();

            let harness = log_snapshot(&stack.harness_calls);
            assert_eq!(harness.len(), 1);
            assert_eq!(harness[0]["action"], json!("execute"));
            assert!(harness[0].get("fs_scope").is_none());
        })
        .await;
    }
}
