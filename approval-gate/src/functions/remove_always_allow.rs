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
        deps.bus.as_ref(),
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
    use std::sync::Arc;

    use super::*;
    use crate::config::WorkerConfig;
    use crate::events::RecordingSink;
    use crate::gate_config::{replace, shared_defaults, GateDefaults};
    use crate::testkit::FakeBus;
    use crate::types::PermissionMode;

    #[tokio::test]
    async fn removes_seed_entries_from_the_materialized_record() {
        let bus = Arc::new(FakeBus::new());
        let _state = bus.with_memory_state();
        let defaults = shared_defaults();
        replace(
            &defaults,
            GateDefaults {
                default_mode: PermissionMode::Auto,
                always_allow_seed: vec!["state::get".into(), "shell::run".into()],
                pending_timeout_ms: 1_800_000,
            },
        );
        let deps = Arc::new(Deps {
            bus,
            sink: Arc::new(RecordingSink::new()),
            defaults,
            cfg: Arc::new(WorkerConfig::default()),
        });
        let res = handle(
            &deps,
            AlwaysAllowMutationRequest {
                session_id: "s_1".into(),
                function_id: "shell::run".into(),
            },
        )
        .await
        .unwrap();
        // The first mutation seeded both entries, then removed one.
        assert_eq!(res.settings.always_allow.len(), 1);
        assert_eq!(res.settings.always_allow[0].function_id, "state::get");

        // Removing an absent entry is a no-op.
        let again = handle(
            &deps,
            AlwaysAllowMutationRequest {
                session_id: "s_1".into(),
                function_id: "never::granted".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(again.settings.always_allow.len(), 1);
    }
}
