//! `approval::gate` — the `pre_trigger` hook (approval-gate.md § The
//! approval::gate hook). Maps `HookInput` → `HookOutput`. Never errors:
//! every failure mode resolves to a fail-closed `deny` so a confused
//! harness cannot interpret an exception as anything but denial (its
//! `on_error: fail_closed` would do the same).
//!
//! Runs inside the harness's at-least-once steps and is idempotent: a
//! redelivered step re-runs the gate for the same `function_call_id`;
//! the pending-record write is keyed on it, so a duplicate hold is a
//! no-op on the existing record (and emits no second `pending_created`).

use super::Deps;
use crate::decision;
use crate::denial::{gate_unavailable_envelope, human_only_denial, permissions_deny_envelope};
use crate::error::ApprovalError;
use crate::pending;
use crate::permissions::Decision;
use crate::redact::redact_for;
use crate::session;
use crate::settings;
use crate::types::{
    now_ms, validate_id, HookCall, HookInput, HookOutput, JsonMap, PendingApprovalRecord,
    PendingKind,
};

pub async fn handle(deps: &Deps, input: HookInput) -> Result<HookOutput, ApprovalError> {
    let Some(call) = input.call.clone() else {
        return Ok(deny(
            "approval-gate received a pre_trigger hook input without a call payload",
        ));
    };

    // 1. Human-only defense — before the settings snapshot, before
    //    config rules, and regardless of mode (even `full`).
    if decision::is_human_only(&call.function_id) {
        let envelope = human_only_denial(&call.function_id, &call.arguments);
        return Ok(deny(&envelope.reason));
    }

    // A call whose ids can't key a pending record can never be held —
    // and a malformed id is a boundary violation anyway. Fail closed.
    if validate_id("session_id", &input.session_id).is_err()
        || validate_id("function_call_id", &call.id).is_err()
    {
        return Ok(deny(&format!(
            "approval-gate cannot evaluate {}: session_id / function_call_id must be non-empty and must not contain \"/\"",
            call.function_id
        )));
    }

    // 2. One config + settings snapshot per call (race-safe). A state
    //    outage degrades to the configuration defaults: safe, because the
    //    default never widens beyond what the deployment configured.
    let cfg = deps.config().await;
    let stored = settings::read_tolerant(deps.iii.as_ref(), &input.session_id).await;
    let (effective, _) = settings::effective(stored, &cfg);

    // 3-5. Mode / allow-list short-circuits.
    if decision::pre_policy_allow(&effective, &call.function_id) {
        return Ok(HookOutput::Continue);
    }

    // 6. Config rules fallback — evaluate the inline permission rules
    //    (first match wins; no match → hold for human approval).
    match cfg
        .permissions()
        .check(&call.function_id, &call.arguments, effective.mode)
    {
        Decision::Allow { .. } => Ok(HookOutput::Continue),
        Decision::Deny {
            rule_id,
            matched_constraint,
        } => {
            let envelope = permissions_deny_envelope(
                &call.function_id,
                &rule_id,
                matched_constraint,
                &call.arguments,
            );
            Ok(deny(&envelope.reason))
        }
        Decision::NeedsApproval => Ok(hold(deps, &input, &call).await),
    }
}

fn deny(reason: &str) -> HookOutput {
    HookOutput::Deny {
        reason: reason.to_string(),
    }
}

/// The hold path. The pending record is written **synchronously, before
/// the hook returns hold** — a held call must never be invisible to the
/// inbox. Write failure → fail-closed deny, never hold blind.
/// `pending_created` emits asynchronously after the record is written —
/// notification fan-out never blocks the trigger hot path.
///
/// Holds never expire: the hook returns `{ decision: "hold" }` with no
/// timeout. A held call stays held until a human resolves it or turn/session
/// cleanup collects it.
async fn hold(deps: &Deps, input: &HookInput, call: &HookCall) -> HookOutput {
    let iii = deps.iii.as_ref();

    // Idempotency: a redelivered step re-holds the same call.
    match pending::get(iii, &input.session_id, &call.id).await {
        Ok(Some(_existing)) => {
            return HookOutput::Hold;
        }
        Ok(None) => {}
        Err(e) => {
            let envelope = gate_unavailable_envelope(
                &call.function_id,
                &format!("pending record read failed: {e}"),
            );
            return deny(&envelope.reason);
        }
    }

    let (session_title, session_description, session_metadata) =
        fetch_session_context(deps, &input.session_id).await;

    let pending_at = now_ms();
    let record = PendingApprovalRecord {
        session_id: input.session_id.clone(),
        turn_id: input.turn_id.clone(),
        function_call_id: call.id.clone(),
        function_id: call.function_id.clone(),
        arguments_excerpt: redact_for(&call.function_id, &call.arguments),
        pending_at,
        session_title,
        session_description,
        session_metadata,
        depth: input.depth,
        assistant_excerpt: None,
        kind: PendingKind::Function,
        access_request: None,
    };

    match pending::put(iii, &record).await {
        Err(e) => {
            let envelope = gate_unavailable_envelope(
                &call.function_id,
                &format!("pending record write failed: {e}"),
            );
            deny(&envelope.reason)
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

/// Best-effort `session::get` — fields are omitted on any failure
/// (session-manager absent, timeout, unknown session). Shared with
/// `filesystem_access_watch` (the same soft-fetch, so the console card has the same
/// session context for both pending-record kinds).
pub(super) async fn fetch_session_context(
    deps: &Deps,
    session_id: &str,
) -> (Option<String>, Option<String>, Option<JsonMap>) {
    let reply = session::get(deps.iii.as_ref(), session_id).await;
    let Ok(reply) = reply else {
        return (None, None, None);
    };
    let Some(meta) = reply.get("meta") else {
        return (None, None, None);
    };
    let title = meta
        .get("title")
        .and_then(serde_json::Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    let description = meta
        .get("description")
        .and_then(serde_json::Value::as_str)
        .filter(|d| !d.is_empty())
        .map(str::to_string);
    let metadata = meta
        .get("metadata")
        .and_then(serde_json::Value::as_object)
        .cloned();
    (title, description, metadata)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::pending::PENDING_SCOPE;
    use crate::settings::SETTINGS_SCOPE;
    use crate::testkit::{
        hook_input as test_hook_input, log_snapshot, settle, state_get, state_set, with_stack,
        BootOpts,
    };
    use crate::types::{ApprovalSettings, PermissionMode};

    fn hook_input(function_id: &str) -> HookInput {
        serde_json::from_value(test_hook_input("s_1", "c_1", function_id)).unwrap()
    }

    async fn seed_settings(
        iii: &iii_sdk::IIIClient,
        session_id: &str,
        settings: &ApprovalSettings,
    ) {
        state_set(
            iii,
            SETTINGS_SCOPE,
            session_id,
            serde_json::to_value(settings).unwrap(),
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn full_mode_allows_without_consulting_rules() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_settings(
                &stack.iii,
                "s_1",
                &ApprovalSettings {
                    mode: PermissionMode::Full,
                    ..Default::default()
                },
            )
            .await;
            let out = handle(&stack.deps, hook_input("shell::run")).await.unwrap();
            assert_eq!(out, HookOutput::Continue);
            assert!(state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1")
                .await
                .is_null());
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn manual_mode_without_grants_holds() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_settings(
                &stack.iii,
                "s_1",
                &ApprovalSettings {
                    mode: PermissionMode::Manual,
                    ..Default::default()
                },
            )
            .await;
            let out = handle(&stack.deps, hook_input("shell::run")).await.unwrap();
            assert!(matches!(out, HookOutput::Hold));
            let record = state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1").await;
            assert_eq!(record["function_id"], json!("shell::run"));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn self_escalation_is_denied_before_everything() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_settings(
                &stack.iii,
                "s_1",
                &ApprovalSettings {
                    mode: PermissionMode::Full,
                    ..Default::default()
                },
            )
            .await;
            for target in ["approval::set-mode", "approval::resolve"] {
                let HookOutput::Deny { reason } =
                    handle(&stack.deps, hook_input(target)).await.unwrap()
                else {
                    panic!("expected deny for {target}");
                };
                assert!(reason.contains("human_only_function"), "{reason}");
            }
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn config_rules_allow_continues() {
        with_stack(BootOpts::allow(), |stack| async move {
            assert_eq!(
                handle(&stack.deps, hook_input("shell::run")).await.unwrap(),
                HookOutput::Continue
            );
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn config_rules_deny_denies() {
        with_stack(BootOpts::deny_function("shell::run"), |stack| async move {
            assert!(matches!(
                handle(&stack.deps, hook_input("shell::run")).await.unwrap(),
                HookOutput::Deny { .. }
            ));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_rules_hold_under_manual_mode() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_settings(
                &stack.iii,
                "s_1",
                &ApprovalSettings {
                    mode: PermissionMode::Manual,
                    ..Default::default()
                },
            )
            .await;
            assert!(matches!(
                handle(&stack.deps, hook_input("shell::run")).await.unwrap(),
                HookOutput::Hold
            ));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn duplicate_hold_is_a_noop_on_the_existing_record() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            seed_settings(
                &stack.iii,
                "s_1",
                &ApprovalSettings {
                    mode: PermissionMode::Manual,
                    ..Default::default()
                },
            )
            .await;
            assert!(matches!(
                handle(&stack.deps, hook_input("shell::run")).await.unwrap(),
                HookOutput::Hold
            ));
            let before = state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1").await;
            assert!(matches!(
                handle(&stack.deps, hook_input("shell::run")).await.unwrap(),
                HookOutput::Hold
            ));
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
    async fn slash_in_ids_fails_closed() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let input: HookInput = serde_json::from_value(json!({
                "session_id": "s/1",
                "turn_id": "t_1",
                "call": { "id": "c_1", "function_id": "shell::run", "arguments": {} }
            }))
            .unwrap();
            assert!(matches!(
                handle(&stack.deps, input).await.unwrap(),
                HookOutput::Deny { .. }
            ));
        })
        .await;
    }
}
