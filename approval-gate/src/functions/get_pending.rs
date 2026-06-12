//! `approval::get_pending` — read one pending record; `null` when
//! resolved or unknown.

use super::Deps;
use crate::error::ApprovalError;
use crate::pending;
use crate::types::{validate_id, GetPendingRequest, GetPendingResponse};

pub async fn handle(
    deps: &Deps,
    req: GetPendingRequest,
) -> Result<Option<GetPendingResponse>, ApprovalError> {
    validate_id("session_id", &req.session_id)?;
    validate_id("function_call_id", &req.function_call_id)?;
    let record = pending::get(
        deps.bus.as_ref(),
        &req.session_id,
        &req.function_call_id,
        deps.cfg.state_timeout_ms,
    )
    .await
    .map_err(|e| ApprovalError::StateUnavailable(format!("pending record read failed: {e}")))?;
    Ok(record.map(|pending| GetPendingResponse { pending }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::config::WorkerConfig;
    use crate::events::RecordingSink;
    use crate::gate_config::shared_defaults;
    use crate::pending::PENDING_SCOPE;
    use crate::testkit::FakeBus;

    #[tokio::test]
    async fn returns_null_for_unknown_and_the_record_when_live() {
        let bus = Arc::new(FakeBus::new());
        let state = bus.with_memory_state();
        let deps = Arc::new(Deps {
            bus,
            sink: Arc::new(RecordingSink::new()),
            defaults: shared_defaults(),
            cfg: Arc::new(WorkerConfig::default()),
        });

        let missing = handle(
            &deps,
            GetPendingRequest {
                session_id: "s_1".into(),
                function_call_id: "c_1".into(),
            },
        )
        .await
        .unwrap();
        assert!(missing.is_none());

        state.seed(
            PENDING_SCOPE,
            "s_1/c_1",
            json!({
                "session_id": "s_1",
                "turn_id": "t_1",
                "function_call_id": "c_1",
                "function_id": "shell::run",
                "pending_at": 1,
                "expires_at": 2,
            }),
        );
        let found = handle(
            &deps,
            GetPendingRequest {
                session_id: "s_1".into(),
                function_call_id: "c_1".into(),
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(found.pending.function_id, "shell::run");
    }
}
