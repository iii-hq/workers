//! `approval::add_always_allow` — curate the session's auto-mode trust
//! list (idempotent add).

use super::Deps;
use crate::error::ApprovalError;
use crate::gate_config::snapshot;
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
    let defaults = snapshot(&deps.defaults);
    let settings = settings::materialize_and(
        deps.bus.as_ref(),
        &req.session_id,
        &defaults,
        deps.cfg.state_timeout_ms,
        |base, now| ApprovalSettings {
            always_allow: with_grant(&base.always_allow, &req.function_id, now),
            ..base
        },
    )
    .await?;
    Ok(SettingsResponse { settings })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::config::WorkerConfig;
    use crate::events::RecordingSink;
    use crate::gate_config::shared_defaults;
    use crate::testkit::FakeBus;

    #[tokio::test]
    async fn add_is_idempotent_on_function_id() {
        let bus = Arc::new(FakeBus::new());
        let _state = bus.with_memory_state();
        let deps = Arc::new(Deps {
            bus,
            sink: Arc::new(RecordingSink::new()),
            defaults: shared_defaults(),
            cfg: Arc::new(WorkerConfig::default()),
        });
        let req = AlwaysAllowMutationRequest {
            session_id: "s_1".into(),
            function_id: "shell::run".into(),
        };
        let first = handle(&deps, req.clone()).await.unwrap();
        assert_eq!(first.settings.always_allow.len(), 1);
        let second = handle(&deps, req).await.unwrap();
        assert_eq!(second.settings.always_allow.len(), 1);
        assert_eq!(first.settings.always_allow, second.settings.always_allow);
    }
}
