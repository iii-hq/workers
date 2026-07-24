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

        // Probe-driven actions fire at completion boundaries: wait for their
        // `after_turns` count, invoke the function, then continue. A reaction
        // the action trips therefore runs while the tracked session is idle,
        // so its turn is the only active one and the scripted (strict-ordinal)
        // router matches it deterministically. Sorted so earlier boundaries
        // fire first; `wait_for_completion_turns` accumulates, so re-waiting a
        // higher count only blocks on the not-yet-seen turns.
        if !self.fixture.probe_actions.is_empty() {
            let mut actions = self.fixture.probe_actions.clone();
            actions.sort_by_key(|a| a.after_turns);
            for action in actions {
                if let Err(error) = services
                    .probe()
                    .wait_for_completion_turns(action.after_turns, deadline)
                    .await
                {
                    if deadline.is_expired() {
                        active.timed_out = true;
                        return Ok(());
                    }
                    return Err(RunError::runner(
                        phase,
                        "wait for probe-action completion boundary",
                        error,
                    ));
                }
                self.fire_probe_action(services, &action, deadline).await?;
            }
        }

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

    /// Invoke a probe action, expanding `{{run_id}}`/`{{session_id}}` in its
    /// payload first. A failed dispatch is a runner error — the scenario's
    /// premise (the reaction it should trip) can't hold without it.
    async fn fire_probe_action(
        &self,
        services: &RunServices,
        action: &crate::fixtures::ProbeAction,
        deadline: Deadline,
    ) -> Result<(), RunError> {
        let mut payload = action.payload.clone();
        crate::expand::Placeholders::new(&self.run_id, &self.session_id)
            .expand_value(&mut payload)
            .map_err(|error| {
                RunError::runner(RunPhase::Await, "expand probe-action payload", error)
            })?;
        services
            .client()
            .call_with_deadline(
                &action.function_id,
                payload,
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
            .map_err(|error| {
                RunError::runner(RunPhase::Await, "fire probe action", anyhow::anyhow!(error))
            })?;
        Ok(())
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
