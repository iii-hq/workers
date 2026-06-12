//! `approval::clear_settings` — drop the session's stored settings record
//! (the session reverts to configuration defaults). Also invoked
//! internally by `approval::on_session_deleted`.

use super::Deps;
use crate::error::ApprovalError;
use crate::settings;
use crate::types::{ClearSettingsRequest, ClearSettingsResponse};

pub async fn handle(
    deps: &Deps,
    req: ClearSettingsRequest,
) -> Result<ClearSettingsResponse, ApprovalError> {
    let cleared = settings::clear(
        deps.bus.as_ref(),
        &req.session_id,
        deps.cfg.state_timeout_ms,
    )
    .await?;
    Ok(ClearSettingsResponse { cleared })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::config::WorkerConfig;
    use crate::events::RecordingSink;
    use crate::functions::set_mode;
    use crate::gate_config::shared_defaults;
    use crate::testkit::FakeBus;
    use crate::types::{PermissionMode, SetModeRequest};

    #[tokio::test]
    async fn clears_a_stored_record_and_tolerates_absence() {
        let bus = Arc::new(FakeBus::new());
        let _state = bus.with_memory_state();
        let deps = Arc::new(Deps {
            bus,
            sink: Arc::new(RecordingSink::new()),
            defaults: shared_defaults(),
            cfg: Arc::new(WorkerConfig::default()),
        });
        let req = ClearSettingsRequest {
            session_id: "s_1".into(),
        };
        assert!(!handle(&deps, req.clone()).await.unwrap().cleared);

        set_mode::handle(
            &deps,
            SetModeRequest {
                session_id: "s_1".into(),
                mode: PermissionMode::Full,
            },
        )
        .await
        .unwrap();
        assert!(handle(&deps, req.clone()).await.unwrap().cleared);
        assert!(!handle(&deps, req).await.unwrap().cleared);
    }
}
