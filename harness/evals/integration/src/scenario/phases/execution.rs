use std::time::Duration;

use serde_json::{json, Value};

use crate::client::DEFAULT_CALL_TIMEOUT_MS;
use crate::deadline::Deadline;
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::services::RunServices;
use crate::stack::Stack;
use crate::types::recorder::RecorderEventKind;
use crate::types::scenario::FaultKind;

use super::super::report::rpc_failure;
use super::super::runner::ScenarioRunner;
use super::super::state::{ActiveTurn, PreparedRun};
use super::{STATUS_POLL_INTERVAL, TARGET_POLL_INTERVAL};

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
                prepared.scenario.send.clone(),
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
                Ok(ActiveTurn::new(deadline, turn_id, value))
            }
            Err(error) => {
                let value = json!({ "error": error });
                self.write_artifact(&prepared.scenario.id, "send-response.json", &value, phase)?;
                Err(rpc_failure(
                    phase,
                    RunErrorKind::Contract,
                    deadline,
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
        deadline
            .poll_until("fault trigger", TARGET_POLL_INTERVAL, || async {
                let events = services.recorder().snapshot(None)?;
                let count = events
                    .iter()
                    .filter(|event| {
                        event.kind == RecorderEventKind::TargetCall
                            && event.function_id == fault.function_id
                    })
                    .count() as u64;
                Ok((count >= fault.after_target_calls).then_some(()))
            })
            .await
            .map_err(|error| {
                let kind = if deadline.is_expired() {
                    RunErrorKind::Timeout
                } else {
                    RunErrorKind::Runner
                };
                RunError::with_source(
                    phase,
                    kind,
                    format!(
                        "fewer than {} target calls observed before fault",
                        fault.after_target_calls
                    ),
                    error,
                )
            })?;

        stack.kill_engine().await.map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Runner,
                "kill engine for fault injection",
                error,
            )
        })?;
        deadline
            .timeout(
                "fault restart delay",
                tokio::time::sleep(Duration::from_millis(fault.restart_delay_ms)),
            )
            .await
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Timeout,
                    "fault restart delay exceeded scenario deadline",
                    error,
                )
            })?;
        stack.respawn_engine().map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Runner,
                "respawn engine after fault injection",
                error,
            )
        })?;
        Ok(())
    }

    pub(in crate::scenario) async fn release(
        &mut self,
        services: &RunServices,
        prepared: &PreparedRun,
        active: &ActiveTurn,
    ) -> Result<(), RunError> {
        let Some(release) = &prepared.scenario.release else {
            return Ok(());
        };
        let phase = RunPhase::Release;
        let deadline = active.deadline;

        deadline
            .poll_until(
                format!("pending call {}", release.function_call_id),
                STATUS_POLL_INTERVAL,
                || async {
                    let status = services
                        .client()
                        .call_with_deadline(
                            "harness::status",
                            json!({ "session_id": self.session_id }),
                            deadline,
                            DEFAULT_CALL_TIMEOUT_MS,
                        )
                        .await;
                    Ok(status.ok().and_then(|status| {
                        status
                            .get("pending_function_calls")
                            .and_then(Value::as_array)
                            .is_some_and(|calls| {
                                calls
                                    .iter()
                                    .any(|call| call.as_str() == Some(&release.function_call_id))
                            })
                            .then_some(())
                    }))
                },
            )
            .await
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Timeout,
                    format!(
                        "call {} never appeared as pending",
                        release.function_call_id
                    ),
                    error,
                )
            })?;

        let response = services
            .client()
            .call_with_deadline(
                "harness::function::resolve",
                json!({
                    "session_id": self.session_id,
                    "turn_id": active.turn_id,
                    "function_call_id": release.function_call_id,
                    "action": release.action,
                }),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await;
        match response {
            Ok(value) => {
                self.write_artifact(
                    &prepared.scenario.id,
                    "resolve-response.json",
                    &value,
                    phase,
                )?;
                Ok(())
            }
            Err(error) => {
                let value = json!({ "error": error });
                self.write_artifact(
                    &prepared.scenario.id,
                    "resolve-response.json",
                    &value,
                    phase,
                )?;
                Err(rpc_failure(
                    phase,
                    RunErrorKind::Contract,
                    deadline,
                    "harness::function::resolve failed",
                    error,
                ))
            }
        }
    }
}
