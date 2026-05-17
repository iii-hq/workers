//! `tearing_down` handler: stop sandbox if any, then transition to stopped.

use harness_types::AgentEvent;
use iii_sdk::{TriggerRequest, III};
use serde_json::json;

use crate::events;
use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};

pub async fn handle(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    let request = persistence::load_run_request(iii, &record.session_id).await;
    if crate::states::assistant::approval_required_enabled(&request) {
        match crate::states::functions::consume_resolved_approval_entries(iii, &record.session_id)
            .await
        {
            Ok(prepared) if !prepared.is_empty() => {
                let executed =
                    crate::states::functions::executed_staging_for_new_prepare_batch(&[]);
                persistence::save_executed_calls(iii, &record.session_id, &executed).await;
                persistence::save_prepared_calls(iii, &record.session_id, &prepared).await;
                record.transition_to(TurnState::FunctionExecute);
                return Ok(());
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(
                    %err,
                    session_id = %record.session_id,
                    "approval::consume failed during teardown; stopping session"
                );
            }
        }
    }

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
