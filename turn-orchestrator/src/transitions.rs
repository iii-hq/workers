//! One-shot transition dispatcher. Drives `record` forward by exactly one
//! state, then returns. Callers persist the new record and decide whether
//! to re-publish `turn::step_requested`.

use iii_sdk::III;

use crate::state::{TurnState, TurnStateRecord};
use crate::states;

pub async fn step(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    match record.state {
        TurnState::Provisioning => states::handle_provisioning(iii, record).await?,
        TurnState::AwaitingAssistant => states::handle_awaiting(iii, record).await?,
        TurnState::AssistantStreaming => states::handle_streaming(iii, record).await?,
        TurnState::AssistantFinished => states::handle_finished(iii, record).await?,
        TurnState::ToolPrepare => states::handle_prepare(iii, record).await?,
        TurnState::ToolExecute => states::handle_execute(iii, record).await?,
        TurnState::ToolFinalize => states::handle_finalize(iii, record).await?,
        TurnState::SteeringCheck => states::handle_steering(iii, record).await?,
        TurnState::TearingDown => states::handle_tearing_down(iii, record).await?,
        TurnState::Stopped => {
            // No-op. Idempotent terminal state.
        }
    }
    Ok(())
}
