//! `approval::add-always-allow` — curate the session's auto-mode trust
//! list (idempotent add).

use super::Deps;
use crate::error::ApprovalError;
use crate::settings::{self, with_grant};
use crate::types::{AlwaysAllowMutationRequest, ApprovalSettings, SettingsResponse};

pub async fn handle(
    deps: &Deps,
    req: AlwaysAllowMutationRequest,
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
                always_allow: with_grant(
                    &base.always_allow,
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
    use super::*;
    use crate::functions::{gate, set_mode};
    use crate::testkit::{BootOpts, hook_input, with_stack};
    use crate::types::{HookInput, HookOutput, PermissionMode, SetModeRequest};

    /// A glob is the operator's to write here, unlike an "approve always"
    /// grant from a held call.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_glob_added_by_hand_matches_in_auto_mode() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            handle(
                &stack.deps,
                AlwaysAllowMutationRequest {
                    session_id: "s_1".into(),
                    namespace: None,
                    function_id: "shell::*".into(),
                },
            )
            .await
            .unwrap();
            let call = |call_id: &str| -> HookInput {
                serde_json::from_value(hook_input("s_1", call_id, "shell::exec")).unwrap()
            };
            assert_eq!(
                gate::handle(&stack.deps, call("c_1")).await.unwrap(),
                HookOutput::Hold
            );
            set_mode::handle(
                &stack.deps,
                SetModeRequest {
                    session_id: "s_1".into(),
                    mode: PermissionMode::Auto,
                },
            )
            .await
            .unwrap();
            assert_eq!(
                gate::handle(&stack.deps, call("c_2")).await.unwrap(),
                HookOutput::Continue
            );
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_is_idempotent_on_function_id() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let req = AlwaysAllowMutationRequest {
                session_id: "s_1".into(),
                namespace: None,
                function_id: "shell::run".into(),
            };
            let first = handle(&stack.deps, req.clone()).await.unwrap();
            assert_eq!(first.settings.always_allow.len(), 1);
            let second = handle(&stack.deps, req).await.unwrap();
            assert_eq!(second.settings.always_allow.len(), 1);
            assert_eq!(first.settings.always_allow, second.settings.always_allow);
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_namespace_is_rejected() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            for ns in ["", "  "] {
                let err = handle(
                    &stack.deps,
                    AlwaysAllowMutationRequest {
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
