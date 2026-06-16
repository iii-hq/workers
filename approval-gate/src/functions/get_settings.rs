//! `approval::get-settings` — read the session's **effective** settings.
//! Never writes (lazy seeding happens on mutation, not on read).

use super::Deps;
use crate::error::ApprovalError;
use crate::gate_config::snapshot;
use crate::settings;
use crate::types::{validate_id, GetSettingsRequest, GetSettingsResponse};

pub async fn handle(
    deps: &Deps,
    req: GetSettingsRequest,
) -> Result<GetSettingsResponse, ApprovalError> {
    validate_id("session_id", &req.session_id)?;
    let defaults = snapshot(&deps.defaults);
    let stored = settings::read_strict(
        deps.iii.as_ref(),
        &req.session_id,
        deps.cfg.state_timeout_ms,
    )
    .await?;
    let (settings, source) = settings::effective(stored, &defaults);
    Ok(GetSettingsResponse { settings, source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::set_mode;
    use crate::settings::SETTINGS_SCOPE;
    use crate::testkit::{state_get, with_stack, BootOpts};
    use crate::types::{PermissionMode, SetModeRequest, SettingsSource};

    #[tokio::test(flavor = "multi_thread")]
    async fn reports_source_defaults_then_stored_and_never_writes_on_read() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let req = GetSettingsRequest {
                session_id: "s_1".into(),
            };

            let before = handle(&stack.deps, req.clone()).await.unwrap();
            assert_eq!(before.source, SettingsSource::Defaults);
            assert_eq!(before.settings.mode, PermissionMode::Manual);
            assert!(state_get(&stack.iii, SETTINGS_SCOPE, "s_1").await.is_null());

            set_mode::handle(
                &stack.deps,
                SetModeRequest {
                    session_id: "s_1".into(),
                    mode: PermissionMode::Full,
                },
            )
            .await
            .unwrap();

            let after = handle(&stack.deps, req).await.unwrap();
            assert_eq!(after.source, SettingsSource::Stored);
            assert_eq!(after.settings.mode, PermissionMode::Full);
        })
        .await;
    }
}
