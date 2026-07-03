//! `approval::remove-always-allow` — remove a function from the session's
//! auto-mode trust list (no-op when absent; seed entries removable like
//! any other — the stored record overrides the deployment seed from first
//! mutation on).

use super::Deps;
use crate::error::ApprovalError;
use crate::settings::{self, without_grant};
use crate::types::{AlwaysAllowMutationRequest, ApprovalSettings, SettingsResponse};

pub async fn handle(
    deps: &Deps,
    req: AlwaysAllowMutationRequest,
) -> Result<SettingsResponse, ApprovalError> {
    let cfg = deps.config().await;
    let settings =
        settings::materialize_and(deps.iii.as_ref(), &req.session_id, &cfg, |base, _now| {
            ApprovalSettings {
                always_allow: without_grant(&base.always_allow, &req.function_id),
                ..base
            }
        })
        .await?;
    Ok(SettingsResponse { settings })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::config::WorkerConfig;
    use crate::testkit::{with_stack, BootOpts};
    use crate::types::PermissionMode;

    #[tokio::test(flavor = "multi_thread")]
    async fn removes_seed_entries_from_the_materialized_record() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            *stack.config.write().await = Arc::new(WorkerConfig {
                default_mode: PermissionMode::Auto,
                rules: vec![
                    serde_json::json!({"function": "state::get", "action": "allow", "modes": ["auto"]}),
                    serde_json::json!({"function": "shell::run", "action": "allow", "modes": ["auto"]}),
                ],
                grant_reask_limit: crate::config::default_grant_reask_limit(),
            });
            let res = handle(
                &stack.deps,
                AlwaysAllowMutationRequest {
                    session_id: "s_1".into(),
                    function_id: "shell::run".into(),
                },
            )
            .await
            .unwrap();
            assert_eq!(res.settings.always_allow.len(), 1);
            assert_eq!(res.settings.always_allow[0].function_id, "state::get");

            let again = handle(
                &stack.deps,
                AlwaysAllowMutationRequest {
                    session_id: "s_1".into(),
                    function_id: "never::granted".into(),
                },
            )
            .await
            .unwrap();
            assert_eq!(again.settings.always_allow.len(), 1);
        })
        .await;
    }
}
