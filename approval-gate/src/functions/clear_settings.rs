//! `approval::clear-settings` — drop the session's stored settings record
//! (the session reverts to configuration defaults). Also invoked
//! internally by `approval::on-session-deleted`.

use super::Deps;
use crate::error::ApprovalError;
use crate::settings;
use crate::types::{ClearSettingsRequest, ClearSettingsResponse};

pub async fn handle(
    deps: &Deps,
    req: ClearSettingsRequest,
) -> Result<ClearSettingsResponse, ApprovalError> {
    let cfg = deps.config().await;
    let cleared = settings::clear(deps.iii.as_ref(), &req.session_id, cfg.state_timeout_ms).await?;
    Ok(ClearSettingsResponse { cleared })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::set_mode;
    use crate::testkit::{with_stack, BootOpts};
    use crate::types::{PermissionMode, SetModeRequest};

    #[tokio::test(flavor = "multi_thread")]
    async fn clears_a_stored_record_and_tolerates_absence() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let req = ClearSettingsRequest {
                session_id: "s_1".into(),
            };
            assert!(!handle(&stack.deps, req.clone()).await.unwrap().cleared);

            set_mode::handle(
                &stack.deps,
                SetModeRequest {
                    session_id: "s_1".into(),
                    mode: PermissionMode::Full,
                },
            )
            .await
            .unwrap();
            assert!(handle(&stack.deps, req.clone()).await.unwrap().cleared);
            assert!(!handle(&stack.deps, req).await.unwrap().cleared);
        })
        .await;
    }
}
