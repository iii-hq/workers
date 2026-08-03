use std::time::Duration;

use serde_json::{json, Value};

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;
use crate::fixtures::ScenarioIntervention;
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::services::RunServices;

use super::super::runner::ScenarioRunner;
use super::super::state::ActiveTurn;

const POLL_INTERVAL: Duration = Duration::from_millis(25);

impl ScenarioRunner<'_> {
    pub(in crate::scenario) async fn intervene(
        &mut self,
        services: &RunServices,
        active: &mut ActiveTurn,
    ) -> Result<(), RunError> {
        let Some(intervention) = self.fixture.intervention.clone() else {
            return Ok(());
        };

        let result = match intervention {
            ScenarioIntervention::StopCancelCascade {
                gate,
                expected_in_flight,
                queued_message,
            } => {
                self.run_stop_cancel_cascade(
                    services,
                    active,
                    &gate,
                    expected_in_flight,
                    &queued_message,
                )
                .await
            }
            ScenarioIntervention::QueuedMessageEditUnqueue {
                gate,
                before_message,
                edit_message,
                edit_replacement,
                remove_message,
                after_message,
            } => {
                self.run_queued_message_edit_unqueue(
                    services,
                    active,
                    &gate,
                    &before_message,
                    &edit_message,
                    &edit_replacement,
                    &remove_message,
                    &after_message,
                )
                .await
            }
            ScenarioIntervention::HeldCallResolve { function_call_id } => {
                self.run_held_call_resolve(services, active, &function_call_id)
                    .await
            }
        };

        match result {
            Ok((control, tree_sessions)) => {
                active.control = control;
                active.tree_sessions = tree_sessions;
                Ok(())
            }
            Err(error) => {
                // A failed premise must never leave the scripted router
                // parked on a gate while the stack is being torn down.
                services.router().release_all_gates();
                Err(error)
            }
        }
    }

    /// INT-013 driver: a scripted pre-trigger hook answered `hold`, parking
    /// the turn in `awaiting_functions`. Prove the no-op resolve gates leave
    /// it parked, then release the held call so the chain resumes after the
    /// holder and the turn completes.
    async fn run_held_call_resolve(
        &self,
        services: &RunServices,
        active: &ActiveTurn,
        function_call_id: &str,
    ) -> Result<(Value, Vec<String>), RunError> {
        let phase = RunPhase::Intervene;
        let deadline = active.deadline;
        let turn_id = active.turn_id.clone().ok_or_else(|| {
            RunError::new(
                phase,
                RunErrorKind::Contract,
                "held-call-resolve requires a turn id from harness::send",
            )
        })?;

        // Wait for the hook hold to park the call durably.
        let parked_status = wait_for_status(services.client(), &self.session_id, deadline, {
            let call_id = function_call_id.to_string();
            move |status| {
                status.get("status").and_then(Value::as_str) == Some("awaiting_functions")
                    && status
                        .get("pending_function_calls")
                        .and_then(Value::as_array)
                        .is_some_and(|pending| {
                            pending.iter().any(|call| {
                                call.get("function_call_id").and_then(Value::as_str)
                                    == Some(call_id.as_str())
                                    || call.get("id").and_then(Value::as_str)
                                        == Some(call_id.as_str())
                            })
                        })
            }
        })
        .await?;

        let resolve = |payload: Value| {
            let client = services.client().clone();
            async move {
                client
                    .call_with_deadline(
                        "harness::function::resolve",
                        payload,
                        deadline,
                        DEFAULT_CALL_TIMEOUT_MS,
                    )
                    .await
                    .map_err(|error| {
                        RunError::with_source(
                            phase,
                            RunErrorKind::Contract,
                            "call harness::function::resolve",
                            anyhow::anyhow!(error),
                        )
                    })
            }
        };

        // No-op gate: a resolve against the wrong turn must not settle the call.
        let wrong_turn = resolve(json!({
            "session_id": self.session_id,
            "turn_id": format!("{turn_id}-bogus"),
            "function_call_id": function_call_id,
            "action": "execute",
        }))
        .await?;
        // No-op gate: an unknown call id must not settle anything either.
        let unknown_call = resolve(json!({
            "session_id": self.session_id,
            "turn_id": turn_id,
            "function_call_id": format!("{function_call_id}-unknown"),
            "action": "execute",
        }))
        .await?;
        for (label, response) in [("wrong turn", &wrong_turn), ("unknown call", &unknown_call)] {
            if response.get("resolved") != Some(&Value::Bool(false)) {
                return Err(RunError::new(
                    phase,
                    RunErrorKind::Contract,
                    format!("{label} resolve was not a no-op: {response}"),
                ));
            }
        }

        // Release: resume the chain after the holder and run the target.
        let execute = resolve(json!({
            "session_id": self.session_id,
            "turn_id": turn_id,
            "function_call_id": function_call_id,
            "action": "execute",
        }))
        .await?;
        if execute.get("resolved") != Some(&Value::Bool(true))
            || execute.get("turn_resumed") != Some(&Value::Bool(true))
        {
            return Err(RunError::new(
                phase,
                RunErrorKind::Contract,
                format!("execute resolve did not resume the turn: {execute}"),
            ));
        }

        let control = json!({
            "kind": "held_call_resolve",
            "parked_status": parked_status,
            "wrong_turn_resolve": wrong_turn,
            "unknown_call_resolve": unknown_call,
            "execute_resolve": execute,
        });
        Ok((control, Vec::new()))
    }

    async fn run_stop_cancel_cascade(
        &self,
        services: &RunServices,
        active: &ActiveTurn,
        gate: &str,
        expected_in_flight: usize,
        queued_message: &str,
    ) -> Result<(Value, Vec<String>), RunError> {
        let phase = RunPhase::Intervene;
        let deadline = active.deadline;
        let root_turn_id = active.turn_id.clone().ok_or_else(|| {
            RunError::new(
                phase,
                RunErrorKind::Contract,
                "stop-cancel-cascade requires a root turn id from harness::send",
            )
        })?;

        services
            .router()
            .wait_for_gate(gate, expected_in_flight, deadline)
            .await
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Contract,
                    format!("wait for {expected_in_flight} calls at scripted gate {gate}"),
                    error,
                )
            })?;

        let root_before = wait_for_status(services.client(), &self.session_id, deadline, {
            let expected_root_turn = root_turn_id.clone();
            move |status| {
                status.get("turn_id").and_then(Value::as_str) == Some(expected_root_turn.as_str())
                    && status.get("status").and_then(Value::as_str) == Some("running")
                    && status
                        .get("children")
                        .and_then(Value::as_array)
                        .is_some_and(|children| children.len() == 2)
            }
        })
        .await?;

        let children = root_before
            .get("children")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                RunError::new(
                    phase,
                    RunErrorKind::Contract,
                    "root status omitted children after the scripted gate opened",
                )
            })?;
        let child_sessions = children
            .iter()
            .map(|child| {
                let session_id = child
                    .get("session_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("child status omitted session_id"))?;
                let turn_id = child
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("child status omitted turn_id"))?;
                Ok::<_, anyhow::Error>((session_id.to_string(), turn_id.to_string()))
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .map_err(|error| {
                RunError::with_source(phase, RunErrorKind::Contract, "read child refs", error)
            })?;

        let child_statuses =
            wait_for_child_statuses(services.client(), &child_sessions, deadline).await?;

        let queued_send = services
            .client()
            .call_with_deadline(
                "harness::send",
                json!({
                    "session_id": self.session_id,
                    "message": queued_message,
                }),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Contract,
                    "queue a message on the running root turn",
                    anyhow::anyhow!(error),
                )
            })?;
        if queued_send.get("queued") != Some(&Value::Bool(true)) {
            return Err(RunError::new(
                phase,
                RunErrorKind::Contract,
                format!("harness::send did not return queued=true: {queued_send}"),
            ));
        }

        let queued_status =
            wait_for_status(services.client(), &self.session_id, deadline, |status| {
                status.get("status").and_then(Value::as_str) == Some("running")
                    && status
                        .get("queued")
                        .and_then(Value::as_array)
                        .is_some_and(|queued| queued.len() == 1)
            })
            .await?;

        let stop_request = json!({
            "session_id": self.session_id,
            "turn_id": root_turn_id,
        });
        let stop_response = services
            .client()
            .call_with_deadline(
                "harness::stop",
                stop_request.clone(),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Contract,
                    "stop the root turn with its explicit turn id",
                    anyhow::anyhow!(error),
                )
            })?;
        if stop_response.get("stopping") != Some(&Value::Bool(true)) {
            return Err(RunError::new(
                phase,
                RunErrorKind::Contract,
                format!("harness::stop did not acknowledge stopping=true: {stop_response}"),
            ));
        }

        services
            .router()
            .release_gate(gate)
            .map_err(|error| RunError::runner(phase, "release scripted router gate", error))?;

        let mut control = json!({
            "kind": "stop_cancel_cascade",
            "gate": {
                "name": gate,
                "expected": expected_in_flight,
                "arrived": expected_in_flight,
                "released": true
            },
            "root_turn_id": root_turn_id,
            "pre_stop_root_status": root_before,
            "pre_stop_child_statuses": child_statuses,
            "queued_send": queued_send,
            "queued_status": queued_status,
            "stop_request": stop_request,
            "stop_response": stop_response,
        });
        control["released"] = Value::Bool(true);

        let mut tree_sessions = vec![self.session_id.clone()];
        tree_sessions.extend(child_sessions.into_iter().map(|(session_id, _)| session_id));
        Ok((control, tree_sessions))
    }

    // The five message labels are deliberately explicit at this fixture seam:
    // each one is a separate hard-gated public-path operation in the evidence.
    #[allow(clippy::too_many_arguments)]
    async fn run_queued_message_edit_unqueue(
        &self,
        services: &RunServices,
        active: &ActiveTurn,
        gate: &str,
        before_message: &str,
        edit_message: &str,
        edit_replacement: &str,
        remove_message: &str,
        after_message: &str,
    ) -> Result<(Value, Vec<String>), RunError> {
        let phase = RunPhase::Intervene;
        let deadline = active.deadline;
        let root_turn_id = active.turn_id.clone().ok_or_else(|| {
            RunError::new(
                phase,
                RunErrorKind::Contract,
                "queued-message-edit-unqueue requires a root turn id from harness::send",
            )
        })?;

        services
            .router()
            .wait_for_gate(gate, 1, deadline)
            .await
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Contract,
                    format!("wait for the first call at scripted gate {gate}"),
                    error,
                )
            })?;

        let pre_intervention_status =
            wait_for_status(services.client(), &self.session_id, deadline, {
                let expected_root_turn = root_turn_id.clone();
                move |status| {
                    status.get("turn_id").and_then(Value::as_str)
                        == Some(expected_root_turn.as_str())
                        && status.get("status").and_then(Value::as_str) == Some("running")
                        && queued_len(status) == 0
                }
            })
            .await?;

        let queue_specs = [
            ("before", before_message, "before"),
            ("edit", edit_message, "edit"),
            ("remove", remove_message, "remove"),
            ("after", after_message, "after"),
        ];
        let mut queued_sends = Vec::with_capacity(queue_specs.len());
        for (label, message, suffix) in queue_specs {
            let idempotency_key = format!("{}:integration-012-{suffix}", self.run_id);
            let response = services
                .client()
                .call_with_deadline(
                    "harness::send",
                    json!({
                        "session_id": self.session_id,
                        "message": message,
                        "idempotency_key": idempotency_key,
                    }),
                    deadline,
                    DEFAULT_CALL_TIMEOUT_MS,
                )
                .await
                .map_err(|error| {
                    RunError::with_source(
                        phase,
                        RunErrorKind::Contract,
                        format!("queue the {label} INT-012 message"),
                        anyhow::anyhow!(error),
                    )
                })?;
            ensure_queued_send(&response, &self.session_id, &root_turn_id, label, phase)?;
            queued_sends.push(json!({
                "label": label,
                "message": message,
                "idempotency_key": idempotency_key,
                "response": response,
            }));
        }

        let pre_edit_status =
            wait_for_status(services.client(), &self.session_id, deadline, |status| {
                status.get("status").and_then(Value::as_str) == Some("running")
                    && queue_contains_all(
                        status,
                        &[before_message, edit_message, remove_message, after_message],
                    )
            })
            .await?;
        let pre_edit_queue = parse_queue(&pre_edit_status).map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Contract,
                "parse the four queued rows before editing",
                error,
            )
        })?;
        if pre_edit_queue.len() != 4 {
            return Err(RunError::new(
                phase,
                RunErrorKind::Contract,
                format!(
                    "expected four queued rows before editing, got {}",
                    pre_edit_queue.len()
                ),
            ));
        }
        let edit_target =
            find_queue_message(&pre_edit_queue, edit_message, "edit").map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Contract,
                    "identify the queued row to edit",
                    error,
                )
            })?;
        let remove_target =
            find_queue_message(&pre_edit_queue, remove_message, "remove").map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Contract,
                    "identify the queued row to unqueue",
                    error,
                )
            })?;
        ensure_client_entry_id(&edit_target, "edit", phase)?;
        ensure_client_entry_id(&remove_target, "remove", phase)?;
        if edit_target.entry_id == remove_target.entry_id {
            return Err(RunError::new(
                phase,
                RunErrorKind::Contract,
                "edit and unqueue targets unexpectedly share an entry_id",
            ));
        }

        let edit_request = json!({
            "session_id": self.session_id,
            "entry_id": edit_target.entry_id,
            "message": edit_replacement,
        });
        let edit_response = services
            .client()
            .call_with_deadline(
                "harness::edit_queued",
                edit_request.clone(),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Contract,
                    "edit the queued message by client-visible entry_id",
                    anyhow::anyhow!(error),
                )
            })?;
        if edit_response.get("updated") != Some(&Value::Bool(true)) {
            return Err(RunError::new(
                phase,
                RunErrorKind::Contract,
                format!("harness::edit_queued did not acknowledge updated=true: {edit_response}"),
            ));
        }

        let post_edit_status =
            wait_for_status(services.client(), &self.session_id, deadline, |status| {
                status.get("status").and_then(Value::as_str) == Some("running")
                    && queue_contains_all(
                        status,
                        &[
                            before_message,
                            edit_replacement,
                            remove_message,
                            after_message,
                        ],
                    )
            })
            .await?;
        let post_edit_queue = parse_queue(&post_edit_status).map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Contract,
                "parse queued rows after editing",
                error,
            )
        })?;
        ensure_edit_preserved_position(
            &pre_edit_queue,
            &post_edit_queue,
            &edit_target.entry_id,
            edit_message,
            edit_replacement,
        )
        .map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Contract,
                "prove that queued edit preserved position and row metadata",
                error,
            )
        })?;

        let unqueue_request = json!({
            "session_id": self.session_id,
            "entry_id": remove_target.entry_id,
        });
        let unqueue_response = services
            .client()
            .call_with_deadline(
                "harness::unqueue",
                unqueue_request.clone(),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Contract,
                    "unqueue the selected message by client-visible entry_id",
                    anyhow::anyhow!(error),
                )
            })?;
        if unqueue_response.get("removed") != Some(&Value::Bool(true)) {
            return Err(RunError::new(
                phase,
                RunErrorKind::Contract,
                format!("harness::unqueue did not acknowledge removed=true: {unqueue_response}"),
            ));
        }

        let post_unqueue_status =
            wait_for_status(services.client(), &self.session_id, deadline, |status| {
                status.get("status").and_then(Value::as_str) == Some("running")
                    && queue_contains_all(
                        status,
                        &[before_message, edit_replacement, after_message],
                    )
                    && !queue_has_entry_id(status, &remove_target.entry_id)
            })
            .await?;
        let post_unqueue_queue = parse_queue(&post_unqueue_status).map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Contract,
                "parse queued rows after unqueue",
                error,
            )
        })?;
        let expected_post_unqueue = post_edit_queue
            .iter()
            .filter(|row| row.entry_id != remove_target.entry_id)
            .cloned()
            .collect::<Vec<_>>();
        if post_unqueue_queue != expected_post_unqueue {
            return Err(RunError::new(
                phase,
                RunErrorKind::Contract,
                format!(
                    "unqueue changed the surviving queue order/content: actual={post_unqueue_queue:?}, expected={expected_post_unqueue:?}"
                ),
            ));
        }

        services
            .router()
            .release_gate(gate)
            .map_err(|error| RunError::runner(phase, "release scripted router gate", error))?;

        let control = json!({
            "kind": "queued-message-edit-unqueue",
            "gate": {
                "name": gate,
                "expected": 1,
                "arrived": 1,
                "released": true,
            },
            "root_turn_id": root_turn_id,
            "pre_intervention_status": pre_intervention_status,
            "queued_sends": queued_sends,
            "pre_edit_status": pre_edit_status,
            "pre_edit_queue": queue_snapshots_json(&pre_edit_queue),
            "edit_target": {
                "entry_id": edit_target.entry_id,
                "internal_id": edit_target.id,
                "position": pre_edit_queue
                    .iter()
                    .position(|row| row.entry_id == edit_target.entry_id),
            },
            "edit_request": edit_request,
            "edit_response": edit_response,
            "post_edit_status": post_edit_status,
            "post_edit_queue": queue_snapshots_json(&post_edit_queue),
            "edit_position_preserved": true,
            "unqueue_target": {
                "entry_id": remove_target.entry_id,
                "internal_id": remove_target.id,
                "position": pre_edit_queue
                    .iter()
                    .position(|row| row.entry_id == remove_target.entry_id),
            },
            "unqueue_request": unqueue_request,
            "unqueue_response": unqueue_response,
            "post_unqueue_status": post_unqueue_status,
            "post_unqueue_queue": queue_snapshots_json(&post_unqueue_queue),
            "unqueue_absent_after_call": !post_unqueue_queue
                .iter()
                .any(|row| row.entry_id == remove_target.entry_id),
            "released": true,
        });
        Ok((control, vec![self.session_id.clone()]))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct QueueSnapshot {
    id: String,
    entry_id: String,
    text: String,
    queued_at: i64,
    origin: Value,
}

fn queued_len(status: &Value) -> usize {
    status
        .get("queued")
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

fn queued_message_texts(status: &Value) -> Option<Vec<String>> {
    status.get("queued")?.as_array().map(|rows| {
        rows.iter()
            .filter_map(|row| row.get("message"))
            .map(crate::evidence_data::message_text)
            .collect()
    })
}

fn queue_contains_all(status: &Value, expected: &[&str]) -> bool {
    let Some(actual) = queued_message_texts(status) else {
        return false;
    };
    actual.len() == expected.len()
        && expected.iter().all(|message| {
            actual
                .iter()
                .filter(|text| text.as_str() == *message)
                .count()
                == 1
        })
}

fn queue_has_entry_id(status: &Value, entry_id: &str) -> bool {
    status
        .get("queued")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter()
                .any(|row| row.get("entry_id").and_then(Value::as_str) == Some(entry_id))
        })
}

fn parse_queue(status: &Value) -> anyhow::Result<Vec<QueueSnapshot>> {
    let rows = status
        .get("queued")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("harness::status omitted queued array: {status}"))?;
    rows.iter()
        .map(|row| {
            let message = row
                .get("message")
                .ok_or_else(|| anyhow::anyhow!("queued row omitted message: {row}"))?;
            anyhow::ensure!(
                message.get("role").and_then(Value::as_str) == Some("user"),
                "queued row is not a user message: {row}"
            );
            Ok(QueueSnapshot {
                id: row
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("queued row omitted internal id: {row}"))?
                    .to_string(),
                entry_id: row
                    .get("entry_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("queued row omitted entry_id: {row}"))?
                    .to_string(),
                text: crate::evidence_data::message_text(message),
                queued_at: row
                    .get("queued_at")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| anyhow::anyhow!("queued row omitted queued_at: {row}"))?,
                origin: row.get("origin").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

fn find_queue_message(
    queue: &[QueueSnapshot],
    text: &str,
    label: &str,
) -> anyhow::Result<QueueSnapshot> {
    let matches = queue
        .iter()
        .filter(|row| row.text == text)
        .collect::<Vec<_>>();
    anyhow::ensure!(
        matches.len() == 1,
        "expected one queued {label} row with text {text:?}, found {}",
        matches.len()
    );
    Ok(matches[0].clone())
}

fn ensure_client_entry_id(
    row: &QueueSnapshot,
    label: &str,
    phase: RunPhase,
) -> Result<(), RunError> {
    if !row.entry_id.starts_with("e_") || row.entry_id.starts_with("q_") {
        return Err(RunError::new(
            phase,
            RunErrorKind::Contract,
            format!(
                "{label} target is not a client-visible entry_id: {}",
                row.entry_id
            ),
        ));
    }
    Ok(())
}

fn ensure_queued_send(
    response: &Value,
    session_id: &str,
    turn_id: &str,
    label: &str,
    phase: RunPhase,
) -> Result<(), RunError> {
    let valid = response.get("accepted") == Some(&Value::Bool(true))
        && response.get("session_id").and_then(Value::as_str) == Some(session_id)
        && response.get("turn_id").and_then(Value::as_str) == Some(turn_id)
        && response.get("merged") == Some(&Value::Bool(true))
        && response.get("queued") == Some(&Value::Bool(true));
    if !valid {
        return Err(RunError::new(
            phase,
            RunErrorKind::Contract,
            format!("harness::send did not queue the {label} message: {response}"),
        ));
    }
    Ok(())
}

fn ensure_edit_preserved_position(
    before: &[QueueSnapshot],
    after: &[QueueSnapshot],
    edit_entry_id: &str,
    old_text: &str,
    new_text: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(before.len() == after.len(), "edit changed queue length");
    for (index, (before_row, after_row)) in before.iter().zip(after).enumerate() {
        anyhow::ensure!(
            before_row.id == after_row.id
                && before_row.entry_id == after_row.entry_id
                && before_row.queued_at == after_row.queued_at
                && before_row.origin == after_row.origin,
            "queue row metadata/order changed at position {index}: before={before_row:?}, after={after_row:?}"
        );
        if before_row.entry_id == edit_entry_id {
            anyhow::ensure!(
                before_row.text == old_text && after_row.text == new_text,
                "edited row content mismatch at position {index}: before={before_row:?}, after={after_row:?}"
            );
        } else {
            anyhow::ensure!(
                before_row.text == after_row.text,
                "non-edited row content changed at position {index}: before={before_row:?}, after={after_row:?}"
            );
        }
    }
    anyhow::ensure!(
        before.iter().position(|row| row.entry_id == edit_entry_id)
            == after.iter().position(|row| row.entry_id == edit_entry_id),
        "edited entry_id moved in the queue"
    );
    Ok(())
}

fn queue_snapshots_json(queue: &[QueueSnapshot]) -> Value {
    Value::Array(
        queue
            .iter()
            .map(|row| {
                json!({
                    "id": row.id,
                    "entry_id": row.entry_id,
                    "text": row.text,
                    "queued_at": row.queued_at,
                    "origin": row.origin,
                })
            })
            .collect(),
    )
}

async fn wait_for_status<F>(
    client: &Client,
    session_id: &str,
    deadline: Deadline,
    ready: F,
) -> Result<Value, RunError>
where
    F: Fn(&Value) -> bool,
{
    let client = client.clone();
    let session_id = session_id.to_string();
    let ready = &ready;
    deadline
        .poll_until("harness::status readiness", POLL_INTERVAL, move || {
            let client = client.clone();
            let session_id = session_id.clone();
            async move {
                let status = client
                    .call_with_deadline(
                        "harness::status",
                        json!({ "session_id": session_id }),
                        deadline,
                        DEFAULT_CALL_TIMEOUT_MS,
                    )
                    .await
                    .map_err(anyhow::Error::msg)?;
                Ok(ready(&status).then_some(status))
            }
        })
        .await
        .map_err(|error| {
            RunError::with_source(
                RunPhase::Intervene,
                RunErrorKind::Contract,
                "wait for harness::status readiness",
                error,
            )
        })
}

async fn wait_for_child_statuses(
    client: &Client,
    children: &[(String, String)],
    deadline: Deadline,
) -> Result<Vec<Value>, RunError> {
    let client = client.clone();
    let children = children.to_vec();
    deadline
        .poll_until(
            "child turns entering running status",
            POLL_INTERVAL,
            move || {
                let client = client.clone();
                let children = children.clone();
                async move {
                    let mut statuses = Vec::with_capacity(children.len());
                    for (session_id, _) in &children {
                        let status = client
                            .call_with_deadline(
                                "harness::status",
                                json!({ "session_id": session_id }),
                                deadline,
                                DEFAULT_CALL_TIMEOUT_MS,
                            )
                            .await
                            .map_err(anyhow::Error::msg)?;
                        statuses.push(status);
                    }
                    let ready = statuses.iter().all(|status| {
                        status.get("status").and_then(Value::as_str) == Some("running")
                    });
                    Ok(ready.then_some(statuses))
                }
            },
        )
        .await
        .map_err(|error| {
            RunError::with_source(
                RunPhase::Intervene,
                RunErrorKind::Contract,
                "wait for child turns entering running status",
                error,
            )
        })
}
