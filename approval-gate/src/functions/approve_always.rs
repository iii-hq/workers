//! `approval::approve_always` — record a per-session "approve always"
//! grant (honoured in **every** mode). Typically called by the console
//! from an approval prompt, immediately before
//! `approval::resolve { decision: "allow" }`.

use super::Deps;
use crate::error::ApprovalError;
use crate::gate_config::snapshot;
use crate::settings::{self, with_grant};
use crate::types::{ApprovalSettings, ApproveAlwaysRequest, SettingsResponse};

pub async fn handle(
    deps: &Deps,
    req: ApproveAlwaysRequest,
) -> Result<SettingsResponse, ApprovalError> {
    if req.function_id.is_empty() {
        return Err(ApprovalError::InvalidPayload(
            "function_id must be a non-empty string".to_string(),
        ));
    }
    let defaults = snapshot(&deps.defaults);
    let settings = settings::materialize_and(
        deps.iii.as_ref(),
        &req.session_id,
        &defaults,
        deps.cfg.state_timeout_ms,
        |base, now| ApprovalSettings {
            approved_always: with_grant(&base.approved_always, &req.function_id, now),
            ..base
        },
    )
    .await?;
    Ok(SettingsResponse { settings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{with_stack, BootOpts};
    use crate::types::GrantedBy;

    #[tokio::test(flavor = "multi_thread")]
    async fn grants_into_approved_always_not_always_allow() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let res = handle(
                &stack.deps,
                ApproveAlwaysRequest {
                    session_id: "s_1".into(),
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
}
