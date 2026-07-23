use std::time::Duration;

use serde_json::json;

use crate::client::DEFAULT_CALL_TIMEOUT_MS;
use crate::deadline::Deadline;
use crate::probe::latest_terminal_observation;
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

        // The event only wakes collection; trace evidence verifies the
        // lifecycle delivery itself. Status remains the durable-state check.
        //
        // A single send can still produce more than one terminal turn when the
        // harness seeds a follow-on turn from it (a reseed after a message
        // parks mid-final-step). Such a scenario declares `terminal_turns(n)`;
        // wait for all n and bind evidence to the latest, mirroring how
        // Playground awaits its externally driven turns.
        let expected = self.fixture.expected_terminal_turns;
        let latest_turn_id = if expected > 1 {
            services
                .probe()
                .wait_for_completion_turns(expected, deadline)
                .await
                .map(|events| latest_terminal_observation(&events).map(|o| o.event.turn_id.clone()))
        } else {
            services
                .probe()
                .wait_for_completion(deadline)
                .await
                .map(|observation| Some(observation.event.turn_id))
        };
        match latest_turn_id {
            // Evidence binds to the LATEST terminal turn: with harness-seeded
            // follow-on turns, Send's own turn id is the first, not the last.
            Ok(Some(turn_id)) if expected > 1 => active.turn_id = Some(turn_id),
            Ok(turn_id) => {
                if active.turn_id.is_none() {
                    active.turn_id = turn_id;
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
                    "wait for harness::turn-completed signal(s)",
                    error,
                ));
            }
        }

        self.confirm_terminal_status(services, active).await
    }

    /// Playground may keep running after its first completed turn. On
    /// shutdown, wait for every turn declared by the fixture, bind evidence
    /// to the latest one, and refresh the durable session status.
    pub(in crate::scenario) async fn refresh_external_completion(
        &mut self,
        services: &RunServices,
        active: &mut ActiveTurn,
    ) -> Result<(), RunError> {
        let deadline = Deadline::after(FINAL_STATUS_TIMEOUT);
        let events = services
            .probe()
            .wait_for_completion_turns(self.fixture.expected_terminal_turns, deadline)
            .await
            .map_err(|error| {
                RunError::runner(
                    RunPhase::Await,
                    "wait for every playground completion signal",
                    error,
                )
            })?;
        if let Some(observation) = latest_terminal_observation(&events) {
            active.turn_id = Some(observation.event.turn_id.clone());
        }
        self.confirm_terminal_status(services, active).await
    }

    async fn confirm_terminal_status(
        &self,
        services: &RunServices,
        active: &mut ActiveTurn,
    ) -> Result<(), RunError> {
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
                    RunPhase::Await,
                    "confirm terminal harness status",
                    anyhow::anyhow!(error),
                )
            })?;
        Ok(())
    }
}
