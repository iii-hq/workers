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
    if req.function_id.is_empty() {
        return Err(ApprovalError::InvalidPayload(
            "function_id must be a non-empty string".to_string(),
        ));
    }
    let cfg = deps.config().await;
    let settings =
        settings::materialize_and(deps.iii.as_ref(), &req.session_id, &cfg, |base, now| {
            ApprovalSettings {
                always_allow: with_grant(&base.always_allow, &req.function_id, now),
                ..base
            }
        })
        .await?;
    Ok(SettingsResponse { settings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{with_stack, BootOpts};

    #[tokio::test(flavor = "multi_thread")]
    async fn add_is_idempotent_on_function_id() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let req = AlwaysAllowMutationRequest {
                session_id: "s_1".into(),
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
}
