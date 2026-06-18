//! `approval::set-mode` — set the session's permission mode. First
//! mutation materializes the settings record from the current
//! configuration defaults (lazy seeding).

use super::Deps;
use crate::error::ApprovalError;
use crate::settings;
use crate::types::{ApprovalSettings, SetModeRequest, SettingsResponse};

pub async fn handle(deps: &Deps, req: SetModeRequest) -> Result<SettingsResponse, ApprovalError> {
    let cfg = deps.config().await;
    let settings =
        settings::materialize_and(deps.iii.as_ref(), &req.session_id, &cfg, |base, now| {
            ApprovalSettings {
                mode: req.mode,
                mode_set_at: now,
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
    use crate::types::PermissionMode;

    #[tokio::test(flavor = "multi_thread")]
    async fn sets_mode_and_stamps_mode_set_at() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let res = handle(
                &stack.deps,
                SetModeRequest {
                    session_id: "s_1".into(),
                    mode: PermissionMode::Auto,
                },
            )
            .await
            .unwrap();
            assert_eq!(res.settings.mode, PermissionMode::Auto);
            assert!(res.settings.mode_set_at > 0);
        })
        .await;
    }
}
