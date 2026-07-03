//! `approval::grant-watch` — the `post_trigger` hook handler
//! (spec-pr3-approval-gate.md § New function `approval::grant-watch`).
//! Watches `shell::*` / `coder::*` dispatch failures for a jail-scope
//! `grant_hint` tail and, when one is found and nothing already excuses it,
//! converts the failure into a held `folder_access` pending approval so the
//! console can ask "allow access to `<dir>`?" instead of surfacing a raw
//! rejection to the model.
//!
//! Bound with `on_error: "fail_open"` (unlike the pre_trigger gate's
//! `fail_closed`): a crashed or absent grant-watch must never turn an
//! already-decided function_result into a stuck call — the ladder's
//! default at every rung, on every failure mode, is `Continue` (deliver
//! the original error to the model unchanged, exactly as if grant-watch
//! were not bound at all).
//!
//! Ladder steps 1-4 (is_error / code / hint parse / workspace guard) are
//! pure and live in `crate::grant::extract_hint`; steps 5-8 need state +
//! harness RPCs and live here.

use super::Deps;
use crate::error::ApprovalError;
use crate::grant::{self, HintOutcome};
use crate::grant_state::{self, AttemptOutcome};
use crate::harness;
use crate::pending;
use crate::redact::redact;
use crate::types::{
    now_ms, validate_id, GrantRequest, HookCall, HookInput, HookOutput, PendingApprovalRecord,
    PendingKind,
};

pub async fn handle(deps: &Deps, input: HookInput) -> Result<HookOutput, ApprovalError> {
    match grant::extract_hint(&input) {
        HintOutcome::NoHint => Ok(HookOutput::Continue),
        HintOutcome::WorkspaceItself(hint) => {
            // A harness stamp hole, not something to prompt about — the
            // session's own workspace should never need a grant.
            tracing::error!(
                session_id = %input.session_id,
                turn_id = %input.turn_id,
                dir = %hint.dir,
                "grant-watch: workspace itself was rejected — harness stamp hole, not prompting"
            );
            Ok(HookOutput::Continue)
        }
        HintOutcome::Candidate(hint) => Ok(evaluate(deps, &input, hint).await),
    }
}

/// Steps 5-8. Any miss (including transport failures — this hook is
/// fail-open) resolves to `Continue`; the caller never sees a held call it
/// cannot explain.
async fn evaluate(deps: &Deps, input: &HookInput, hint: GrantRequest) -> HookOutput {
    let Some(call) = input.call.clone() else {
        return HookOutput::Continue;
    };
    if validate_id("session_id", &input.session_id).is_err()
        || validate_id("function_call_id", &call.id).is_err()
    {
        return HookOutput::Continue;
    }

    let iii = deps.iii.as_ref();

    // 5. Already granted: an existing durable grant covering `hint.dir`
    // means shell/coder still rejected it — a store/stamp skew, not
    // something a re-ask fixes. Deliver the error, don't loop. A harness
    // without this control plane (old harness, not deployed) also lands
    // here: fail-open, standing the watch down entirely.
    match harness::workspace_grants(iii, &input.session_id).await {
        Ok(roots) => {
            if roots
                .iter()
                .any(|root| grant::dir_contains(root, &hint.dir))
            {
                return HookOutput::Continue;
            }
        }
        Err(e) => {
            tracing::debug!(
                session_id = %input.session_id,
                error = %e,
                "grant-watch: harness::workspace::grants unavailable; standing down"
            );
            return HookOutput::Continue;
        }
    }

    // 6. Denied memory: the user already said no to this dir this session.
    let denied = grant_state::read_denied(iii, &input.session_id).await;
    if denied.iter().any(|d| grant::dir_contains(d, &hint.dir)) {
        return HookOutput::Continue;
    }

    // Idempotency: a redelivered post_trigger step re-holds the same call
    // without re-counting an attempt or re-emitting — mirrors gate.rs's
    // hold(). This only applies when the existing record is a redelivery of
    // THIS SAME failure (its grant_request matches `hint`); a record left
    // behind for a different failure (e.g. an orphan from a delete that
    // failed after resolve — see resolve.rs's "retry or purge will collect
    // it" path) must not be silently reused for a genuinely new dir/path,
    // or the attempt cap (step 7) and the console never learn about it.
    match pending::get(iii, &input.session_id, &call.id).await {
        Ok(Some(existing)) if existing.grant_request.as_ref() == Some(&hint) => {
            return HookOutput::Hold;
        }
        Ok(Some(_stale)) => {
            // Clear the stale record so the write below (step 8) is a clean
            // insert rather than racing `hold`'s own duplicate-write
            // detection, which would otherwise suppress the re-emit this
            // genuinely new failure needs.
            if let Err(e) = pending::delete_with_gate(iii, &input.session_id, &call.id).await {
                tracing::warn!(
                    session_id = %input.session_id,
                    function_call_id = %call.id,
                    error = %e,
                    "grant-watch: stale pending record cleanup failed; continuing (fail-open)"
                );
                return HookOutput::Continue;
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(
                session_id = %input.session_id,
                function_call_id = %call.id,
                error = %e,
                "grant-watch: pending record read failed; continuing (fail-open)"
            );
            return HookOutput::Continue;
        }
    }

    // 7. Attempt cap: increment BEFORE holding so a redelivery of the
    // eventual hold (above) never double-counts, but a genuinely new
    // failure for the same call (a different path this time) does.
    let cfg = deps.config().await;
    let bumped = grant_state::check_and_bump(
        iii,
        &input.session_id,
        &call.id,
        &input.turn_id,
        cfg.grant_reask_limit,
    )
    .await;
    match bumped {
        Ok(AttemptOutcome::Capped) => return HookOutput::Continue,
        Ok(AttemptOutcome::Allowed(_)) => {}
        Err(e) => {
            tracing::warn!(
                session_id = %input.session_id,
                function_call_id = %call.id,
                error = %e,
                "grant-watch: attempt-cap write failed; continuing (fail-open)"
            );
            return HookOutput::Continue;
        }
    }

    hold(deps, input, &call, hint).await
}

/// Step 8: write the pending record synchronously, THEN emit
/// `pending_created` fire-and-forget, THEN return hold — the exact same
/// ordering discipline as `gate::hold`.
async fn hold(deps: &Deps, input: &HookInput, call: &HookCall, hint: GrantRequest) -> HookOutput {
    let iii = deps.iii.as_ref();
    let (session_title, session_description, session_metadata) =
        super::gate::fetch_session_context(deps, &input.session_id).await;

    let record = PendingApprovalRecord {
        session_id: input.session_id.clone(),
        turn_id: input.turn_id.clone(),
        function_call_id: call.id.clone(),
        function_id: call.function_id.clone(),
        arguments_excerpt: redact(&call.arguments),
        pending_at: now_ms(),
        session_title,
        session_description,
        session_metadata,
        depth: input.depth,
        assistant_excerpt: None,
        kind: PendingKind::FolderAccess,
        grant_request: Some(hint),
    };

    match pending::put(iii, &record).await {
        Err(e) => {
            tracing::warn!(
                session_id = %input.session_id,
                function_call_id = %call.id,
                error = %e,
                "grant-watch: pending record write failed; continuing (fail-open, mirrors gate's fail-closed-toward-delivery discipline)"
            );
            HookOutput::Continue
        }
        // Lost a write race with a concurrent duplicate hold: the first
        // writer's record (and emission) stands.
        Ok(Some(_prior)) => HookOutput::Hold,
        Ok(None) => {
            let sink = deps.sink.clone();
            tokio::spawn(async move {
                sink.pending_created(&record).await;
            });
            HookOutput::Hold
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::pending::PENDING_SCOPE;
    use crate::testkit::{
        log_snapshot, settle, state_get, state_set, with_stack, BootOpts, TestStack,
    };

    fn jail_message(dir: &str) -> String {
        format!(
            "outside jail grant_hint={{\"v\":1,\"dir\":\"{dir}\",\"path\":\"{dir}/x\",\"code\":\"S215\"}}"
        )
    }

    fn post_trigger_input(
        session_id: &str,
        call_id: &str,
        function_id: &str,
        is_error: bool,
        code: Option<&str>,
        message: &str,
        metadata: Option<serde_json::Value>,
    ) -> HookInput {
        let mut base = json!({
            "point": "post_trigger",
            "session_id": session_id,
            "turn_id": "t_1",
            "step": 3,
            "depth": 0,
            "call": { "id": call_id, "function_id": function_id, "arguments": { "path": "/x" } },
            "result": {
                "function_call_id": call_id,
                "function_id": function_id,
                "content": [{ "type": "text", "text": message }],
                "is_error": is_error,
                "details": { "error": code.map(|c| json!({ "code": c, "message": message })) },
            },
        });
        if let Some(metadata) = metadata {
            base["metadata"] = metadata;
        }
        serde_json::from_value(base).unwrap()
    }

    fn jail_input(session_id: &str, call_id: &str, dir: &str) -> HookInput {
        post_trigger_input(
            session_id,
            call_id,
            "shell::fs::read",
            true,
            Some("S215"),
            &jail_message(dir),
            None,
        )
    }

    /// Register the fakes `evaluate()` depends on beyond the standard
    /// testkit boot: `harness::workspace::grants` (empty by default) and
    /// `harness::workspace::grant`. Both log to `stack.harness_calls` under
    /// a `{"__fn": ...}` tag so a test can distinguish them from
    /// `harness::function::resolve` log entries if needed.
    fn register_workspace_fakes(stack: &TestStack, existing_roots: Vec<String>) {
        let iii = stack.iii.clone();
        let roots = existing_roots;
        iii.register_function(
            "harness::workspace::grants",
            iii_sdk::RegisterFunction::new_async(move |_req: serde_json::Value| {
                let roots = roots.clone();
                async move { Ok::<_, iii_sdk::errors::Error>(json!({ "roots": roots })) }
            }),
        );
        let log = stack.harness_calls.clone();
        iii.register_function(
            "harness::workspace::grant",
            iii_sdk::RegisterFunction::new_async(move |req: serde_json::Value| {
                let log = log.clone();
                async move {
                    crate::testkit::log_push(&log, req.clone());
                    let root = req.get("root").cloned().unwrap_or(serde_json::Value::Null);
                    Ok::<_, iii_sdk::errors::Error>(json!({
                        "session_id": req.get("session_id"),
                        "roots": [root],
                    }))
                }
            }),
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn non_error_result_continues() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            register_workspace_fakes(&stack, vec![]);
            let input = post_trigger_input(
                "s_1",
                "c_1",
                "shell::fs::read",
                false,
                Some("S215"),
                &jail_message("/a/b"),
                None,
            );
            assert_eq!(
                handle(&stack.deps, input).await.unwrap(),
                HookOutput::Continue
            );
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn non_jail_scope_code_continues() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            register_workspace_fakes(&stack, vec![]);
            let input = post_trigger_input(
                "s_1",
                "c_1",
                "shell::exec",
                true,
                Some("S499"),
                &jail_message("/a/b"),
                None,
            );
            assert_eq!(
                handle(&stack.deps, input).await.unwrap(),
                HookOutput::Continue
            );
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_hint_in_message_continues() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            register_workspace_fakes(&stack, vec![]);
            let input = post_trigger_input(
                "s_1",
                "c_1",
                "shell::exec",
                true,
                Some("S215"),
                "outside the jail",
                None,
            );
            assert_eq!(
                handle(&stack.deps, input).await.unwrap(),
                HookOutput::Continue
            );
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn workspace_itself_continues() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            register_workspace_fakes(&stack, vec![]);
            let input = post_trigger_input(
                "s_1",
                "c_1",
                "shell::fs::read",
                true,
                Some("S215"),
                &jail_message("/ws/sub"),
                Some(json!({ "working_dir": "/ws" })),
            );
            assert_eq!(
                handle(&stack.deps, input).await.unwrap(),
                HookOutput::Continue
            );
            assert!(state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1")
                .await
                .is_null());
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn already_granted_continues() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            register_workspace_fakes(&stack, vec!["/a".to_string()]);
            let input = jail_input("s_1", "c_1", "/a/b");
            assert_eq!(
                handle(&stack.deps, input).await.unwrap(),
                HookOutput::Continue
            );
            assert!(state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1")
                .await
                .is_null());
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_harness_workspace_support_stands_down() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            // No `harness::workspace::grants` fake registered at all — the
            // trigger simply doesn't exist.
            let input = jail_input("s_1", "c_1", "/a/b");
            assert_eq!(
                handle(&stack.deps, input).await.unwrap(),
                HookOutput::Continue
            );
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn denied_memory_continues() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            register_workspace_fakes(&stack, vec![]);
            state_set(
                &stack.iii,
                grant_state::DENIED_SCOPE,
                "s_1",
                json!(["/a/b"]),
            )
            .await;
            let input = jail_input("s_1", "c_1", "/a/b/c");
            assert_eq!(
                handle(&stack.deps, input).await.unwrap(),
                HookOutput::Continue
            );
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn attempt_cap_continues_after_the_configured_limit() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            register_workspace_fakes(&stack, vec![]);
            let mut cfg = (*stack.config.read().await).as_ref().clone();
            cfg.grant_reask_limit = 1;
            crate::configuration::apply_config(&stack.config, cfg).await;

            // 1st attempt for c_1: allowed, holds.
            let out1 = handle(&stack.deps, jail_input("s_1", "c_1", "/a/b"))
                .await
                .unwrap();
            assert_eq!(out1, HookOutput::Hold);

            // Resolve it away (deny) so the pending record no longer exists
            // and a 2nd genuinely-new failure for the SAME call is possible.
            crate::testkit::call(
                &stack.iii,
                "approval::resolve",
                json!({ "session_id": "s_1", "function_call_id": "c_1", "decision": "deny" }),
            )
            .await
            .unwrap();

            // 2nd attempt (a different path failing on the same call_id):
            // the cap (limit=1) is already met -> Continue.
            let out2 = handle(&stack.deps, jail_input("s_1", "c_1", "/c/d"))
                .await
                .unwrap();
            assert_eq!(out2, HookOutput::Continue);
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn happy_path_holds_a_folder_access_record() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            register_workspace_fakes(&stack, vec![]);
            let out = handle(&stack.deps, jail_input("s_1", "c_1", "/a/b"))
                .await
                .unwrap();
            assert_eq!(out, HookOutput::Hold);

            let record = state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1").await;
            assert_eq!(record["kind"], json!("folder_access"));
            assert_eq!(record["grant_request"]["dir"], json!("/a/b"));
            assert_eq!(record["grant_request"]["error_code"], json!("S215"));
            assert_eq!(record["function_id"], json!("shell::fs::read"));

            assert!(
                crate::testkit::wait_for(3_000, || { !log_snapshot(&stack.created).is_empty() })
                    .await
            );
            let created = log_snapshot(&stack.created);
            assert_eq!(created[0]["kind"], json!("folder_access"));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn duplicate_post_trigger_delivery_is_a_noop_on_the_existing_record() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            register_workspace_fakes(&stack, vec![]);
            let out1 = handle(&stack.deps, jail_input("s_1", "c_1", "/a/b"))
                .await
                .unwrap();
            assert_eq!(out1, HookOutput::Hold);
            let before = state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1").await;

            let out2 = handle(&stack.deps, jail_input("s_1", "c_1", "/a/b"))
                .await
                .unwrap();
            assert_eq!(out2, HookOutput::Hold);
            assert_eq!(
                state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1").await,
                before
            );

            settle().await;
            assert_eq!(log_snapshot(&stack.created).len(), 1);
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stale_record_for_a_different_dir_is_replaced_not_reused() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            register_workspace_fakes(&stack, vec![]);

            // A prior hold for c_1 on /a/b left an orphaned record behind
            // (simulating a resolve whose delete-after-resolve failed —
            // resolve.rs's "retry or purge will collect it" path).
            let out1 = handle(&stack.deps, jail_input("s_1", "c_1", "/a/b"))
                .await
                .unwrap();
            assert_eq!(out1, HookOutput::Hold);
            settle().await;
            assert_eq!(log_snapshot(&stack.created).len(), 1);

            // The same call_id now fails again on a genuinely different
            // path. The stale /a/b record must not be reused as-is: the
            // new failure gets its own record, attempt count, and emission.
            let out2 = handle(&stack.deps, jail_input("s_1", "c_1", "/c/d"))
                .await
                .unwrap();
            assert_eq!(out2, HookOutput::Hold);

            let record = state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1").await;
            assert_eq!(record["grant_request"]["dir"], json!("/c/d"));

            settle().await;
            let created = log_snapshot(&stack.created);
            assert_eq!(created.len(), 2, "the new dir gets its own pending_created");
            assert_eq!(created[1]["grant_request"]["dir"], json!("/c/d"));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_ids_fail_open_to_continue() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            register_workspace_fakes(&stack, vec![]);
            let input: HookInput = serde_json::from_value(json!({
                "point": "post_trigger",
                "session_id": "s/1",
                "turn_id": "t_1",
                "call": { "id": "c_1", "function_id": "shell::fs::read", "arguments": {} },
                "result": {
                    "function_call_id": "c_1",
                    "function_id": "shell::fs::read",
                    "is_error": true,
                    "content": [],
                    "details": { "error": { "code": "S215", "message": jail_message("/a/b") } },
                },
            }))
            .unwrap();
            assert_eq!(
                handle(&stack.deps, input).await.unwrap(),
                HookOutput::Continue
            );
        })
        .await;
    }
}
