use std::time::Duration;

use serde_json::{json, Value};

use crate::deadline::Deadline;
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::services::RunServices;

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
}
