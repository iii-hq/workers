//! INT-041 and INT-042 — `harness::stop` on a session parked on a wake.
//!
//! A parked session's turn already ended, so a stop has nothing to cancel, and the
//! wake's deadline is still ahead. Every one-shot wake has one, so this decides what
//! a stop means for a parked session:
//!
//! * INT-041, a person's Stop (`quiet_deadlines`): the deadline starts no turn, and
//!   the session ends cancelled with `terminal: true` for whoever waits on it. That
//!   run finishing at all proves the event, since the waiter only counts terminal ones.
//! * INT-042, an agent's stop: the session still ends on its next wake, the deadline
//!   notice included. Agents rely on that to run their own fallback.

use serde_json::{Value, json};

use super::ScenarioDriver;
use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use crate::evidence_data::RunEvidence;
use crate::fixtures::ScenarioFixture;

const OBSERVE: &str = "harness::observe";
const STOP: &str = "harness::stop";
const SCOPE: &str = "e2e-041";
const KEY: &str = "never";
const MESSAGE: &str = "Arm a wake and park.";
/// Comfortably after the stop, which lands right after the park.
const EXPIRES_IN_MS: u64 = 8_000;

pub(super) fn quiet_deadlines() -> ScenarioFixture {
    parked_then_stopped(
        "INT-041",
        "stop-parked-quiet-deadline",
        "A person's Stop on a parked session: its wake's deadline starts no turn and the \
         exchange ends terminal.",
        json!({ "session_id": "{{session_id}}", "quiet_deadlines": true }),
    )
    .terminal_turn_statuses(["completed"])
    .parked_completions(1)
    .terminal_outside_traces("cancelled")
    .expect_traces(1)
    .verify(|run| {
        run.expect_assistant_texts(["armed and parked"])?;
        anyhow::ensure!(
            notifications(run).is_empty(),
            "a stopped session must not hear about its wake's deadline: {:?}",
            notifications(run)
        );
        expect_no_wake_left(run)?;
        // The turn's output was only its wiring, so it must not read as the result.
        anyhow::ensure!(
            run.status.get("status").and_then(Value::as_str) == Some("cancelled"),
            "a stopped session must end cancelled: {}",
            run.status
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

pub(super) fn agent_stop() -> ScenarioFixture {
    parked_then_stopped(
        "INT-042",
        "stop-parked-next-wake",
        "An agent's stop on a parked session leaves it to end on its next wake, the deadline \
         notice included.",
        json!({ "session_id": "{{session_id}}" }),
    )
    .terminal_turn_statuses(["completed", "completed"])
    .parked_completions(1)
    .expect_traces(2)
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user" }),
                    ])
                    .tools_exact_after_controls([OBSERVE], [record().tool()]),
            )
            .respond(Response::text("deadline noted", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["armed and parked", "deadline noted"])?;
        let notices = notifications(run);
        anyhow::ensure!(
            notices.len() == 1 && notices[0].contains("wake expired unfired"),
            "the deadline must still reach a session an agent stopped: {notices:?}"
        );
        expect_no_wake_left(run)?;
        run.expect_no_duplicate_messages()
    })
    .build()
}

/// Exposes a native tool so every generation pins `tools_exact` as its siblings do.
fn record() -> ControlledFunction {
    ControlledFunction::new("{{run_id}}::record", "Record one value.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        }))
        .returns_text("recorded")
}

/// A turn that arms a wake on a key nothing writes and parks, then the stop.
fn parked_then_stopped(id: &str, slug: &str, description: &str, stop: Value) -> Scenario {
    let model = Model::scripted("fixture-model");
    let record = record();
    Scenario::new(id, slug, description, ScenarioDriver::Direct, model.clone())
        .send(
            Send::message(MESSAGE)
                .idempotency_key(&format!("{{{{run_id}}}}:{slug}"))
                .allow_id(OBSERVE)
                // An agent observes only what its policy allows.
                .allow_id("state::on-changed")
                .allow_function(&record),
        )
        .probe_after_parked(1, STOP, stop)
        .function(record.clone())
        .generation(
            Generation::new(1)
                .expect(
                    Request::new()
                        .turn_request_step(0)
                        .system_prompt_sha256("{{system_prompt_sha256}}")
                        .messages_exact([Message::user(MESSAGE)])
                        .tools_exact_after_controls([OBSERVE], [record.tool()]),
                )
                .respond(Response::function_call_raw(
                    "call-arm",
                    OBSERVE,
                    json!({
                        "function_id": "state::on-changed",
                        "filters": { "input.scope": { "$eq": SCOPE }, "input.key": { "$eq": KEY } },
                        "once": true,
                        "label": "never-written",
                        "lifecycle": { "expires_in_ms": EXPIRES_IN_MS }
                    }),
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
                            json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-arm", "function_id": OBSERVE }
                        ] }),
                            json!({ "role": "function_result", "function_call_id": "call-arm",
                                "is_error": false }),
                        ])
                        .tools_exact_after_controls([OBSERVE], [record.tool()]),
                )
                .respond(Response::text("armed and parked", 10, 2)),
        )
}

fn notifications(run: &RunEvidence) -> Vec<String> {
    run.transcript
        .iter()
        .filter_map(|item| item.get("message"))
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .map(|message| {
            message
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .filter(|text| text.contains("[notification]"))
        .collect()
}

fn expect_no_wake_left(run: &RunEvidence) -> anyhow::Result<()> {
    anyhow::ensure!(
        run.status.get("expects_wake").and_then(Value::as_bool) == Some(false),
        "the deadline must leave no wake expected: {}",
        run.status
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stop_after_the_park_is_the_only_probe() {
        for fixture in [quiet_deadlines(), agent_stop()] {
            fixture.validate().unwrap();
            assert_eq!(fixture.probe_actions.len(), 1);
            assert_eq!(fixture.probe_actions[0].after_parked, Some(1));
            assert_eq!(fixture.probe_actions[0].function_id, STOP);
        }
    }
}
