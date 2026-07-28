//! `approval::evaluate` — what would `approval::gate` decide for this call,
//! WITHOUT deciding it.
//!
//! The gate is a `pre_trigger` hook: it can only answer while a turn is
//! dispatching, and a `needs_approval` verdict WRITES a pending record for a
//! human to resolve. Callers that need the verdict ahead of time — the
//! harness, when an agent asks to bind a trigger to an ordinary function —
//! cannot use it: a trigger-fired call dispatches with worker authority
//! outside the turn loop, so it never reaches the gate at all, and probing
//! the gate itself would spawn phantom pendings nobody asked for.
//!
//! This runs the SAME decision ladder as [`super::gate`] (human-only defense,
//! mode/always-allow short-circuit, then the configured rules) and reports the
//! verdict. It never writes: no pending record, no settings seeding.
//!
//! SECURITY: the point of the probe is the invariant "a reaction may not do
//! what a direct call could not do unapproved". A binding is only safe when
//! this answers `allow`; `needs_approval` means the deployment wants a human
//! in the loop, and a fired trigger has no way to ask one.

use super::Deps;
use crate::decision;
use crate::error::ApprovalError;
use crate::permissions::Decision;
use crate::settings;
use crate::types::{validate_id, EvaluateRequest, EvaluateResponse, EvaluateVerdict};

pub async fn handle(deps: &Deps, req: EvaluateRequest) -> Result<EvaluateResponse, ApprovalError> {
    validate_id("session_id", &req.session_id)?;
    if req.function_id.trim().is_empty() {
        return Err(ApprovalError::InvalidPayload(
            "function_id must be a non-empty function id".into(),
        ));
    }

    // 1. Human-only defense — same precedence as the gate: before settings,
    //    before config rules, regardless of mode.
    if decision::is_human_only(&req.function_id) {
        return Ok(EvaluateResponse {
            verdict: EvaluateVerdict::Deny,
            reason: Some(format!(
                "{} is human-only and can never be called by an agent",
                req.function_id
            )),
        });
    }

    let cfg = deps.config().await;
    // Tolerant read like the gate's: a state outage degrades to configuration
    // defaults rather than failing the probe open.
    let stored = settings::read_tolerant(deps.iii.as_ref(), &req.session_id).await;
    let (effective, _) = settings::effective(stored, &cfg);

    if decision::pre_policy_allow(&effective, &req.function_id) {
        return Ok(EvaluateResponse {
            verdict: EvaluateVerdict::Allow,
            reason: None,
        });
    }

    let arguments = req.arguments.clone().unwrap_or_else(|| serde_json::json!({}));
    let verdict = match cfg
        .permissions()
        .check(&req.function_id, &arguments, effective.mode)
    {
        Decision::Allow { .. } => EvaluateResponse {
            verdict: EvaluateVerdict::Allow,
            reason: None,
        },
        Decision::Deny { rule_id, .. } => EvaluateResponse {
            verdict: EvaluateVerdict::Deny,
            reason: Some(format!(
                "{} is denied by permission rule {rule_id}",
                req.function_id
            )),
        },
        Decision::NeedsApproval => EvaluateResponse {
            verdict: EvaluateVerdict::NeedsApproval,
            reason: Some(format!(
                "{} requires human approval per this deployment's permission rules",
                req.function_id
            )),
        },
    };
    Ok(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::{add_always_allow, approve_always, set_mode};
    use crate::settings::SETTINGS_SCOPE;
    use crate::testkit::{state_get, with_stack, BootOpts};
    use crate::types::{
        AlwaysAllowMutationRequest, ApproveAlwaysRequest, PermissionMode, SetModeRequest,
    };

    fn req(function_id: &str) -> EvaluateRequest {
        EvaluateRequest {
            session_id: "s_1".into(),
            function_id: function_id.into(),
            arguments: None,
        }
    }

    /// The probe must report the same ladder the gate walks, and must leave no
    /// trace — a pending record written by a probe is an approval prompt no
    /// human asked for.
    #[tokio::test(flavor = "multi_thread")]
    async fn reports_the_gate_ladder_and_never_writes() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            // Unlisted under a needs-approval deployment: the verdict a
            // trigger binding must be refused on.
            let out = handle(&stack.deps, req("fp::pipe")).await.unwrap();
            assert_eq!(out.verdict, EvaluateVerdict::NeedsApproval);
            assert!(out.reason.unwrap().contains("human approval"));

            // Probing wrote nothing: no settings record, no pending.
            assert!(state_get(&stack.iii, SETTINGS_SCOPE, "s_1").await.is_null());

            // An auto-mode trust-list entry does NOT flip a manual-mode
            // session: `always_allow` only short-circuits in auto mode, so the
            // binding stays refused here.
            add_always_allow::handle(
                &stack.deps,
                AlwaysAllowMutationRequest {
                    session_id: "s_1".into(),
                    function_id: "fp::pipe".into(),
                },
            )
            .await
            .unwrap();
            assert_eq!(
                handle(&stack.deps, req("fp::pipe")).await.unwrap().verdict,
                EvaluateVerdict::NeedsApproval
            );

            // A remembered human decision ("approve always") applies in EVERY
            // mode — a human was in the loop, so the binding is safe.
            approve_always::handle(
                &stack.deps,
                ApproveAlwaysRequest {
                    session_id: "s_1".into(),
                    function_id: "fp::pipe".into(),
                },
            )
            .await
            .unwrap();
            let out = handle(&stack.deps, req("fp::pipe")).await.unwrap();
            assert_eq!(out.verdict, EvaluateVerdict::Allow);
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn human_only_targets_are_denied_even_in_full_mode() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            set_mode::handle(
                &stack.deps,
                SetModeRequest {
                    session_id: "s_1".into(),
                    mode: PermissionMode::Full,
                },
            )
            .await
            .unwrap();
            // Full mode allows ordinary calls…
            assert_eq!(
                handle(&stack.deps, req("fp::pipe")).await.unwrap().verdict,
                EvaluateVerdict::Allow
            );
            // …but never a human-only one.
            let out = handle(&stack.deps, req("approval::resolve")).await.unwrap();
            assert_eq!(out.verdict, EvaluateVerdict::Deny);
            assert!(out.reason.unwrap().contains("human-only"));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_requests_are_rejected() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            assert!(handle(&stack.deps, req("  ")).await.is_err());
            let bad_session = EvaluateRequest {
                session_id: String::new(),
                function_id: "fp::pipe".into(),
                arguments: None,
            };
            assert!(handle(&stack.deps, bad_session).await.is_err());
        })
        .await;
    }
}
