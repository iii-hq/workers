//! `approval::remove_always_allow` — remove a function from the session's
//! auto-mode trust list (no-op when absent; seed entries removable like
//! any other — the stored record overrides the deployment seed from first
//! mutation on).

use super::Deps;
use crate::error::ApprovalError;
use crate::gate_config::snapshot;
use crate::settings::{self, without_grant};
use crate::types::{AlwaysAllowMutationRequest, ApprovalSettings, SettingsResponse};

pub async fn handle(
    deps: &Deps,
    req: AlwaysAllowMutationRequest,
) -> Result<SettingsResponse, ApprovalError> {
    let defaults = snapshot(&deps.defaults);
    let settings = settings::materialize_and(
        deps.iii.as_ref(),
        &req.session_id,
        &defaults,
        deps.cfg.state_timeout_ms,
        |base, _now| ApprovalSettings {
            always_allow: without_grant(&base.always_allow, &req.function_id),
            ..base
        },
    )
    .await?;
    Ok(SettingsResponse { settings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate_config::{replace, GateDefaults};
    use crate::testkit::{with_stack, BootOpts};
    use crate::types::PermissionMode;

    #[tokio::test(flavor = "multi_thread")]
    async fn removes_seed_entries_from_the_materialized_record() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            replace(
                &stack.defaults,
                GateDefaults {
                    default_mode: PermissionMode::Auto,
                    always_allow_seed: vec!["state::get".into(), "shell::run".into()],
                    pending_timeout_ms: 1_800_000,
                },
            );
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
