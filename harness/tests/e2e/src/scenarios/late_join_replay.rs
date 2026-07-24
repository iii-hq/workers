//! E2E-009 — a join predecessor registered AFTER its watched session already
//! completed still fires: the harness delivers a catch-up completion (the
//! level-triggered join barrier).
//!
//! The race it gates (rctest5 attempt 4): an orchestrator's join-predecessor
//! registration was delayed one round-trip (a rejected sibling forced a
//! re-registration) and the watched writer sessions finished in that window —
//! edge-triggered `turn-completed` bindings then starve forever, the
//! finalizer never spawns, and the run parks with its report unwritten.
//!
//! Choreography (all deterministic, no sleeps):
//! 1. Turn 1 registers (a) a task reaction that spawns the WORKER session on
//!    a state key, and (b) a call-mode reaction on the worker's
//!    `turn-completed` — the recorder call that PROVES the completion event
//!    already fired.
//! 2. The probe writes the state key → the worker session runs one turn and
//!    completes → recorder call #1.
//! 3. Only after call #1 (`probe_after_calls`) does the probe steer turn 2
//!    into the tracked session, which registers the LATE join predecessor on
//!    the worker's `turn-completed`. The completion is strictly in the past.
//! 4. Without the catch-up replay this join can never fire (timeout). With
//!    it, the registration detects the terminally-completed session, delivers
//!    the synthetic completion, the single-key join completes, and its
//!    call-mode downstream makes recorder call #2.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const SCOPE: &str = "e2e-009";
const SPAWN_KEY: &str = "spawn-worker";
const MESSAGE_2: &str = "The worker already finished. Register the late join predecessor now.";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "E2E-009";
    const MESSAGE: &str = "Spawn the worker and watch its completion.";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new("{{run_id}}::record", "Record one event.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": true
        }))
        .returns_text("recorded");

    // (a) The worker: a task reaction pinned to its own session.
    let register_worker_spawn = json!({
        "trigger_type": "state",
        "config": { "scope": SCOPE, "key": SPAWN_KEY },
        "function_id": "harness::react",
        "metadata": {
            "task": "Say that the work is done, then stop.",
            "session_id": "{{run_id}}-worker"
        }
    });
    // (b) The completion witness: a call reaction that makes recorder call #1
    // the moment the worker's turn-completed event fires.
    let register_completion_witness = json!({
        "trigger_type": "harness::turn-completed",
        "config": { "session_id": "{{run_id}}-worker" },
        "function_id": "harness::react",
        "metadata": { "call": { "function_id": "{{run_id}}::record" } }
    });
    // (c) The LATE join predecessor, registered strictly after the completion:
    // its single-key join fires the call-mode downstream (recorder call #2)
    // only if the harness replays the already-past completion.
    let register_late_join = json!({
        "trigger_type": "harness::turn-completed",
        "config": { "session_id": "{{run_id}}-worker" },
        "function_id": "harness::react",
        "metadata": {
            "join": { "id": "{{run_id}}-late-join", "expect": ["w"], "key": "w" },
            "call": { "function_id": "{{run_id}}::record" }
        }
    });

    Scenario::new(
        ID,
        "late-join-predecessor-replay",
        "A join predecessor registered after its watched session completed receives a catch-up fire.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:e2e-009")
            .allow_id(REGISTER)
            .allow_function(&record),
    )
    // Turn 1 (arm) + turn 2 (the probe-steered late registration).
    .terminal_turn_statuses(["completed", "completed"])
    // Worker + witness + downstream run in untracked flows; only Send's own
    // trace and the probe-steered second send trace group under the session.
    .expect_traces(2)
    // Call #1 = the completion witness; call #2 = the replayed join's
    // downstream. #2 exists only through the catch-up path.
    .await_target_calls(2)
    .probe_after(
        1,
        "state::set",
        json!({ "scope": SCOPE, "key": SPAWN_KEY, "value": { "go": true } }),
    )
    // Fires only after recorder call #1 proved the worker's completion event
    // is strictly in the past — the late registration cannot race it.
    .probe_after_calls(
        1,
        1,
        "harness::send",
        json!({ "session_id": "{{session_id}}", "message": MESSAGE_2 }),
    )
    .function(record.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-spawn",
                REGISTER,
                register_worker_spawn,
                8,
                4,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result", "function_call_id": "call-spawn",
                                "is_error": false }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-witness",
                REGISTER,
                register_completion_witness,
                8,
                4,
            )),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(2)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result", "function_call_id": "call-witness",
                                "is_error": false }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("armed", 10, 2)),
    )
    // The worker session's single turn, spawned by the probe's state write.
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(".")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("work done", 8, 2)),
    )
    // Turn 2, steered by the probe strictly after recorder call #1.
    .generation(
        Generation::new(5)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user", "content": [
                            { "type": "text", "text": MESSAGE_2 }
                        ] }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-late-join",
                REGISTER,
                register_late_join,
                8,
                4,
            )),
    )
    .generation(
        Generation::new(6)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result", "function_call_id": "call-late-join",
                                "is_error": false }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("late join armed", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["armed", "late join armed"])?;
        run.expect_target_calls(2)?;
        // Call #1: the live completion event (the witness). Proves ordering.
        let witness = run.target_calls[0]
            .get("event")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        anyhow::ensure!(
            witness.get("terminal").and_then(serde_json::Value::as_bool) == Some(true)
                && witness.get("__late_subscription_replay").is_none(),
            "call #1 must be the LIVE completion event: {witness}"
        );
        // Call #2: the joined downstream — its results map carries the
        // replayed completion under the join key, stamped as a replay.
        let replay = run.target_calls[1]
            .pointer("/event/results/w")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        anyhow::ensure!(
            replay.get("__late_subscription_replay").and_then(serde_json::Value::as_bool)
                == Some(true),
            "call #2 must carry the catch-up replay under join key `w`: {:?}",
            run.target_calls[1]
        );
        anyhow::ensure!(
            replay.get("status").and_then(serde_json::Value::as_str) == Some("completed")
                && replay.get("terminal").and_then(serde_json::Value::as_bool) == Some(true),
            "replayed event must carry the completion shape: {replay}"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_gates_the_late_registration_on_the_witness_call() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.await_target_calls, Some(2));
        assert_eq!(fixture.expected_traces(), 2);
        assert_eq!(fixture.probe_actions.len(), 2);
        assert_eq!(fixture.probe_actions[0].after_target_calls, None);
        assert_eq!(fixture.probe_actions[1].after_target_calls, Some(1));
    }
}
