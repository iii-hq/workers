use std::time::Duration;

use serde_json::{json, Value};

use crate::deadline::Deadline;
use crate::discovery;
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::services::RunServices;
use crate::stack::Stack;
use crate::types::scenario::FaultKind;

use super::super::report::rpc_failure;
use super::super::runner::ScenarioRunner;
use super::super::state::{ActiveTurn, PreparedRun};

const SEND_TIMEOUT_MS: u64 = 30_000;

impl ScenarioRunner<'_> {
    pub(in crate::scenario) async fn send(
        &mut self,
        services: &RunServices,
        prepared: &PreparedRun,
    ) -> Result<ActiveTurn, RunError> {
        let phase = RunPhase::Send;
        let deadline = Deadline::after(Duration::from_millis(
            prepared.scenario.deadlines.scenario_ms,
        ));
        let trace_generation = services.probe().current_trace_generation();

        self.write_artifact(
            &prepared.scenario.id,
            "request.json",
            &prepared.scenario.send,
            phase,
        )?;
        let response = services
            .client()
            .call_with_deadline(
                "harness::send",
                serde_json::to_value(&prepared.scenario.send).map_err(|error| {
                    RunError::runner(phase, "serialize compiled harness::send request", error)
                })?,
                deadline,
                SEND_TIMEOUT_MS,
            )
            .await;
        match response {
            Ok(value) => {
                self.write_artifact(&prepared.scenario.id, "send-response.json", &value, phase)?;
                let turn_id = value
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .map(String::from);
                Ok(ActiveTurn::new(deadline, turn_id, value, trace_generation))
            }
            Err(error) => {
                self.write_artifact(
                    &prepared.scenario.id,
                    "send-response.json",
                    &json!({ "error": error }),
                    phase,
                )?;
                Err(rpc_failure(
                    phase,
                    RunErrorKind::Contract,
                    "harness::send failed",
                    error,
                ))
            }
        }
    }

    pub(in crate::scenario) async fn fault(
        &mut self,
        stack: &mut Stack,
        services: &RunServices,
        prepared: &PreparedRun,
        active: &ActiveTurn,
    ) -> Result<(), RunError> {
        let Some(fault) = &prepared.scenario.fault else {
            return Ok(());
        };
        let phase = RunPhase::Fault;
        let deadline = active.deadline;

        let FaultKind::EngineSigkill = fault.kind;
        let expected_calls = usize::try_from(fault.after_target_calls)
            .map_err(|error| RunError::runner(phase, "convert fault target call count", error))?;
        if let Err(error) = services
            .probe()
            .wait_for_target_calls(expected_calls, deadline)
            .await
        {
            services.probe().release_target_response();
            let kind = if deadline.is_expired() {
                RunErrorKind::Contract
            } else {
                RunErrorKind::Runner
            };
            return Err(RunError::with_source(
                phase,
                kind,
                format!(
                    "fewer than {} controlled-function calls observed before fault",
                    fault.after_target_calls
                ),
                error,
            ));
        }

        let kill_result = stack.kill_engine().await;
        services.probe().release_target_response();
        kill_result
            .map_err(|error| RunError::runner(phase, "kill engine for fault injection", error))?;
        services
            .probe()
            .forget_observer_bindings()
            .map_err(|error| {
                RunError::runner(phase, "discard pre-restart observer bindings", error)
            })?;

        deadline
            .timeout(
                "fault restart delay",
                tokio::time::sleep(Duration::from_millis(fault.restart_delay_ms)),
            )
            .await
            .map_err(|error| {
                RunError::runner(
                    phase,
                    "fault restart delay exceeded scenario deadline",
                    error,
                )
            })?;
        stack
            .respawn_engine()
            .map_err(|error| RunError::runner(phase, "respawn engine after fault", error))?;

        // Bind through the probe connection first: its replayed function
        // registrations are ordered before this acknowledged RPC, and
        // recoverable lifecycle bindings park until the harness trigger type
        // returns. This minimizes the post-restart completion race.
        services
            .probe()
            .bind_observers(&self.session_id, deadline)
            .await
            .map_err(|error| {
                RunError::runner(phase, "restore observers after engine restart", error)
            })?;

        let mut required = discovery::TURN_SURFACE.to_vec();
        required.push("router::chat");
        if let Some(target) = &prepared.scenario.target {
            required.push(target.function_id.as_str());
        }
        discovery::wait_for_functions(services.client(), &required, deadline)
            .await
            .map_err(|error| {
                RunError::runner(
                    phase,
                    "wait for turn dependencies after engine restart",
                    error,
                )
            })?;
        services
            .probe()
            .confirm_completion_binding(&self.session_id, deadline)
            .await
            .map_err(|error| {
                RunError::runner(
                    phase,
                    "confirm completion observer after engine restart",
                    error,
                )
            })?;
        Ok(())
    }
}
