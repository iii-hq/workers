use std::time::Duration;

use serde_json::{json, Value};

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;
use crate::discovery;
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::services::RunServices;
use crate::stack::Stack;
use crate::types::recorder::RecorderEventKind;
use crate::types::scenario::{CompiledScenarioV1, FaultKind};

use super::super::report::rpc_failure;
use super::super::runner::ScenarioRunner;
use super::super::state::{ActiveTurn, PreparedRun};

const SEND_TIMEOUT_MS: u64 = 30_000;
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const TARGET_POLL_INTERVAL: Duration = Duration::from_millis(50);

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
                Ok(ActiveTurn::new(deadline, turn_id, value))
            }
            Err(error) => {
                let value = json!({ "error": error });
                self.write_artifact(&prepared.scenario.id, "send-response.json", &value, phase)?;
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
        let observed = deadline
            .poll_until("fault trigger", TARGET_POLL_INTERVAL, || async {
                let events = services.recorder().snapshot()?;
                let count = events
                    .iter()
                    .filter(|event| {
                        event.kind == RecorderEventKind::TargetCall
                            && event.function_id == fault.function_id
                    })
                    .count() as u64;
                Ok((count >= fault.after_target_calls).then_some(()))
            })
            .await;
        if let Err(error) = observed {
            services
                .recorder()
                .release_response(&fault.function_id)
                .map_err(|release_error| {
                    RunError::runner(
                        phase,
                        "release controlled response gate after fault trigger failure",
                        release_error,
                    )
                })?;
            let kind = if deadline.is_expired() {
                RunErrorKind::Contract
            } else {
                RunErrorKind::Runner
            };
            return Err(RunError::with_source(
                phase,
                kind,
                format!(
                    "fewer than {} target calls observed before fault",
                    fault.after_target_calls
                ),
                error,
            ));
        }

        let kill_result = stack.kill_engine().await;
        services
            .recorder()
            .release_response(&fault.function_id)
            .map_err(|error| {
                RunError::runner(phase, "release controlled response gate after fault", error)
            })?;
        kill_result
            .map_err(|error| RunError::runner(phase, "kill engine for fault injection", error))?;
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
        stack.respawn_engine().map_err(|error| {
            RunError::runner(phase, "respawn engine after fault injection", error)
        })?;
        self.restore_after_engine_restart(services, prepared, deadline)
            .await?;
        Ok(())
    }

    async fn restore_after_engine_restart(
        &mut self,
        services: &RunServices,
        prepared: &PreparedRun,
        deadline: Deadline,
    ) -> Result<(), RunError> {
        let phase = RunPhase::Fault;
        let scenario = &prepared.scenario;

        discovery::wait_for_functions(services.client(), discovery::TURN_SURFACE, deadline)
            .await
            .map_err(|error| {
                RunError::runner(
                    phase,
                    "wait for turn function surface after engine restart",
                    error,
                )
            })?;
        discovery::wait_for_trigger_types(
            services.client(),
            &["harness::turn-started", "harness::turn-completed"],
            deadline,
        )
        .await
        .map_err(|error| {
            RunError::runner(
                phase,
                "wait for harness trigger types after engine restart",
                error,
            )
        })?;

        let expected_bindings = expected_trigger_bindings(scenario, &self.session_id);
        let registered = registered_trigger_snapshot(services.client(), deadline)
            .await
            .map_err(|error| {
                RunError::runner(
                    phase,
                    "inspect trigger bindings after engine restart",
                    error,
                )
            })?;

        for (index, expected) in expected_bindings.iter().enumerate() {
            match registered_trigger_count(&registered, expected) {
                1 => {}
                0 if index == 0 => services
                    .recorder()
                    .bind_lifecycle(
                        scenario.recorder.lifecycle.trigger_type.as_str(),
                        &self.session_id,
                    )
                    .await
                    .map_err(|error| {
                        RunError::runner(
                            phase,
                            "restore lifecycle binding after engine restart",
                            error,
                        )
                    })?,
                0 => {
                    let binding = &scenario.bindings[index - 1];
                    services
                        .recorder()
                        .bind(
                            &binding.trigger_type,
                            &binding.function_id,
                            binding.config.clone(),
                        )
                        .await
                        .map_err(|error| {
                            RunError::runner(
                                phase,
                                format!(
                                    "restore trigger {} -> {} after engine restart",
                                    binding.trigger_type, binding.function_id
                                ),
                                error,
                            )
                        })?;
                }
                count => {
                    return Err(RunError::new(
                        phase,
                        RunErrorKind::Runner,
                        format!(
                            "trigger binding {} -> {} appeared {count} times after engine restart",
                            expected.trigger_type, expected.function_id
                        ),
                    ));
                }
            }
        }
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
                    RunErrorKind::Contract,
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
                    "harness::function::resolve failed",
                    error,
                ))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ExpectedTriggerBinding {
    trigger_type: String,
    function_id: String,
    config: Value,
}

fn expected_trigger_bindings(
    scenario: &CompiledScenarioV1,
    session_id: &str,
) -> Vec<ExpectedTriggerBinding> {
    std::iter::once(ExpectedTriggerBinding {
        trigger_type: scenario
            .recorder
            .lifecycle
            .trigger_type
            .as_str()
            .to_string(),
        function_id: "integration-recorder::lifecycle".to_string(),
        config: json!({ "session_id": session_id }),
    })
    .chain(
        scenario
            .bindings
            .iter()
            .map(|binding| ExpectedTriggerBinding {
                trigger_type: binding.trigger_type.clone(),
                function_id: binding.function_id.clone(),
                config: binding.config.clone(),
            }),
    )
    .collect()
}

async fn registered_trigger_snapshot(client: &Client, deadline: Deadline) -> anyhow::Result<Value> {
    client
        .call_with_deadline(
            "engine::registered-triggers::list",
            json!({ "include_internal": true }),
            deadline,
            DEFAULT_CALL_TIMEOUT_MS,
        )
        .await
        .map_err(anyhow::Error::msg)
}

fn registered_trigger_count(listed: &Value, expected: &ExpectedTriggerBinding) -> usize {
    listed
        .get("registered_triggers")
        .and_then(Value::as_array)
        .or_else(|| listed.as_array())
        .into_iter()
        .flatten()
        .filter(|row| {
            row.get("function_id").and_then(Value::as_str) == Some(expected.function_id.as_str())
                && row.get("trigger_type").and_then(Value::as_str)
                    == Some(expected.trigger_type.as_str())
                && row.get("config") == Some(&expected.config)
        })
        .count()
}
