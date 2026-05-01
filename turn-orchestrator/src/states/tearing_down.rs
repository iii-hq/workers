//! `tearing_down` handler: stop sandbox if any, then transition to stopped.

use harness_types::AgentEvent;
use iii_sdk::{TriggerRequest, III};
use serde_json::json;

use crate::events;
use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};

pub async fn handle(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    if let Some(sandbox_id) = persistence::load_sandbox_id(iii, &record.session_id).await {
        if let Err(e) = iii
            .trigger(TriggerRequest {
                function_id: "sandbox::stop".into(),
                payload: json!({ "sandbox_id": sandbox_id, "wait": true }),
                action: None,
                timeout_ms: Some(60_000),
            })
            .await
        {
            tracing::warn!(error = %e, sandbox_id = %sandbox_id, "sandbox::stop failed during teardown");
        }
    }

    let messages = persistence::load_messages(iii, &record.session_id).await;
    events::emit(iii, &record.session_id, &AgentEvent::AgentEnd { messages }).await;

    record.transition_to(TurnState::Stopped);
    Ok(())
}
