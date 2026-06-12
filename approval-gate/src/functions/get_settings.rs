//! `approval::get_settings` — read the session's **effective** settings.
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
        deps.bus.as_ref(),
        &req.session_id,
        deps.cfg.state_timeout_ms,
    )
    .await?;
    let (settings, source) = settings::effective(stored, &defaults);
    Ok(GetSettingsResponse { settings, source })
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
    use crate::types::{PermissionMode, SetModeRequest, SettingsSource};

    #[tokio::test]
    async fn reports_source_defaults_then_stored_and_never_writes_on_read() {
        let bus = Arc::new(FakeBus::new());
        let state = bus.with_memory_state();
        let deps = Arc::new(Deps {
            bus,
            sink: Arc::new(RecordingSink::new()),
            defaults: shared_defaults(),
            cfg: Arc::new(WorkerConfig::default()),
        });
        let req = GetSettingsRequest {
            session_id: "s_1".into(),
        };

        let before = handle(&deps, req.clone()).await.unwrap();
        assert_eq!(before.source, SettingsSource::Defaults);
        assert_eq!(before.settings.mode, PermissionMode::Manual);
        assert!(state.is_empty());

        set_mode::handle(
            &deps,
            SetModeRequest {
                session_id: "s_1".into(),
                mode: PermissionMode::Full,
            },
        )
        .await
        .unwrap();

        let after = handle(&deps, req).await.unwrap();
        assert_eq!(after.source, SettingsSource::Stored);
        assert_eq!(after.settings.mode, PermissionMode::Full);
    }
}
