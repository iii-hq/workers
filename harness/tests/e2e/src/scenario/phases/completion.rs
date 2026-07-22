use std::time::Duration;

use serde_json::json;

use crate::client::DEFAULT_CALL_TIMEOUT_MS;
use crate::deadline::Deadline;
use crate::runtime::{RunError, RunPhase};
use crate::services::RunServices;

use super::super::runner::ScenarioRunner;
use super::super::state::ActiveTurn;

const FINAL_STATUS_TIMEOUT: Duration = Duration::from_secs(10);

impl ScenarioRunner<'_> {
    pub(in crate::scenario) async fn r#await(
        &mut self,
        services: &RunServices,
        active: &mut ActiveTurn,
    ) -> Result<(), RunError> {
        let phase = RunPhase::Await;
        let deadline = active.deadline;

        // Completion is event-driven: Arm bound harness::turn-completed to the
        // recorder. Once it arrives, make one status call as the durable-state
        // confirmation checked by the floor.
        match services.recorder().wait_for_lifecycle(deadline).await {
            Ok(event) => {
                if active.turn_id.is_none() {
                    active.turn_id = event
                        .payload
                        .get("turn_id")
                        .and_then(serde_json::Value::as_str)
                        .map(String::from);
                }
            }
            Err(error) if deadline.is_expired() => {
                active.timed_out = true;
                tracing::error!(
                    target: "harness_integration::scenario",
                    "await timed out: {error:#}"
                );
                return Ok(());
            }
            Err(error) => {
                return Err(RunError::runner(
                    phase,
                    "wait for harness::turn-completed delivery",
                    error,
                ));
            }
        }

        let evidence_deadline = Deadline::after(FINAL_STATUS_TIMEOUT);
        active.final_status = services
            .client()
            .call_with_deadline(
                "harness::status",
                json!({ "session_id": self.session_id }),
                evidence_deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
            .map_err(|error| {
                RunError::runner(
                    phase,
                    "confirm terminal harness status",
                    anyhow::anyhow!(error),
                )
            })?;
        Ok(())
    }
}
