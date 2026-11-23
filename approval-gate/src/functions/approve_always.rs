//! `approval::approve-always` — record a per-session "approve always"
//! grant (honoured in **every** mode). Typically called by the console
//! from an approval prompt, immediately before
//! `approval::resolve { decision: "allow" }`.

use super::Deps;
use crate::error::ApprovalError;
use crate::settings::{self, with_grant};
use crate::types::{ApprovalSettings, ApproveAlwaysRequest, SettingsResponse};

pub async fn handle(
    deps: &Deps,
    req: ApproveAlwaysRequest,
) -> Result<SettingsResponse, ApprovalError> {
    super::validate_namespace(req.namespace.as_deref())?;
    if req.function_id.is_empty() {
        return Err(ApprovalError::InvalidPayload(
            "function_id must be a non-empty string".to_string(),
        ));
    }
    let cfg = deps.config().await;
    let settings =
        settings::materialize_and(deps.iii.as_ref(), &req.session_id, &cfg, |base, now| {
            ApprovalSettings {
                approved_always: with_grant(
                    &base.approved_always,
                    req.namespace.as_deref(),
                    &req.function_id,
                    now,
                ),
                ..base
            }
        })
        .await?;
    Ok(SettingsResponse { settings })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::functions::{evaluate, gate};
    use crate::pending::PENDING_SCOPE;
    use crate::testkit::{BootOpts, hook_input, state_get, with_stack};
    use crate::types::{EvaluateRequest, EvaluateVerdict, GrantedBy, HookInput, HookOutput};

    fn call(call_id: &str, function_id: &str, namespace: Option<&str>) -> HookInput {
        let mut v = hook_input("s_1", call_id, function_id);
        if let Some(ns) = namespace {
            v["call"]["namespace"] = json!(ns);
            v["call"]["implicit_namespace"] = json!("default");
        }
        serde_json::from_value(v).unwrap()
    }

    fn grant(namespace: Option<&str>, function_id: &str) -> ApproveAlwaysRequest {
        ApproveAlwaysRequest {
            session_id: "s_1".into(),
            namespace: namespace.map(str::to_string),
            function_id: function_id.into(),
        }
    }

    /// "Always" on a held call whose id holds a `*` grants that literal id,
    /// never every function the `*` would match.
    #[tokio::test(flavor = "multi_thread")]
    async fn always_on_a_star_id_does_not_allow_the_functions_it_would_match() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            handle(&stack.deps, grant(None, "shell::*")).await.unwrap();

            let out = gate::handle(&stack.deps, call("c_1", "shell::exec", None))
                .await
                .unwrap();
            assert_eq!(out, HookOutput::Hold);
            let held = state_get(&stack.iii, PENDING_SCOPE, "s_1/c_1").await;
            assert_eq!(held["function_id"], json!("shell::exec"));

            // The `harness::observe` target probe answers the same.
            let probe = evaluate::handle(
                &stack.deps,
                EvaluateRequest {
                    session_id: "s_1".into(),
                    function_id: "shell::exec".into(),
                    namespace: None,
                    implicit_namespace: None,
                    arguments: None,
                },
            )
            .await
            .unwrap();
            assert_eq!(probe.verdict, EvaluateVerdict::NeedsApproval);

            let out = gate::handle(&stack.deps, call("c_2", "shell::*", None))
                .await
                .unwrap();
            assert_eq!(out, HookOutput::Continue);
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_exact_grant_allows_its_id_in_its_namespace_only() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            handle(&stack.deps, grant(Some("laptop"), "shell::exec"))
                .await
                .unwrap();
            let out = gate::handle(&stack.deps, call("c_1", "shell::exec", Some("laptop")))
                .await
                .unwrap();
            assert_eq!(out, HookOutput::Continue);
            let out = gate::handle(&stack.deps, call("c_2", "shell::exec", Some("desktop")))
                .await
                .unwrap();
            assert_eq!(out, HookOutput::Hold);
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grants_into_approved_always_not_always_allow() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let res = handle(
                &stack.deps,
                ApproveAlwaysRequest {
                    session_id: "s_1".into(),
                    namespace: None,
                    function_id: "shell::run".into(),
                },
            )
            .await
            .unwrap();
            assert!(res.settings.always_allow.is_empty());
            assert_eq!(res.settings.approved_always.len(), 1);
            assert_eq!(
                res.settings.approved_always[0].granted_by,
                GrantedBy::UserClick
            );
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_namespace_is_rejected() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            for ns in ["", "  "] {
                let err = handle(
                    &stack.deps,
                    ApproveAlwaysRequest {
                        session_id: "s_1".into(),
                        namespace: Some(ns.into()),
                        function_id: "shell::run".into(),
                    },
                )
                .await
                .unwrap_err();
                assert!(matches!(err, ApprovalError::InvalidPayload(_)));
            }
        })
        .await;
    }
}
