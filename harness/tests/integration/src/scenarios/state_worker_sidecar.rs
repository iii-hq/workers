//! INT-006 — a wake bound to a `state` key fires through the standalone
//! state worker, carrying the registration-metadata sidecar.
//!
//! This is the cross-worker seam every rctest run turned on (MOT-4209): the
//! state worker's fan-out must invoke the harness delivery hop WITH the
//! binding's `__binding` metadata, or the fire resolves nothing and silently
//! no-ops. This run's stack includes the real `state` worker (see
//! `stack/config.rs`); the engine builtin is disabled, so the standalone
//! worker owns the `state` trigger type exactly as in production.
//!
//! Shape note: the binding is a STANDING wake bounded by `max_fires: 1`
//! rather than a `once` wake. A once-wake would PARK turn 1 (no terminal
//! turn), and the probe only fires after a terminal turn exists — standing
//! keeps turn 1 terminal, the probe writes the key while the session is
//! idle, and the lifecycle bound still retires the binding after its single
//! delivery.

use serde_json::{json, Value};

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const SCOPE: &str = "e2e-006";
const KEY: &str = "go";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-006";
    const MESSAGE: &str = "Arm a state-key wake.";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new("{{run_id}}::record", "Record one value.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        }))
        .returns_text("recorded");

    // A wake (no `function_id`): the fire notifies THIS session. Standing +
    // max_fires keeps turn 1 terminal for the probe boundary (see module doc).
    let register_args = json!({
        "trigger_type": "state",
        "config": { "scope": SCOPE, "key": KEY },
        "once": false,
        "lifecycle": { "max_fires": 1 }
    });

    Scenario::new(
        ID,
        "state-worker-sidecar",
        "A state-key wake fires through the standalone state worker with its metadata sidecar.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-006")
            .allow_id(REGISTER)
            .allow_function(&record),
    )
    // Main turn (1) + the state-fired wake turn (2).
    .terminal_turn_statuses(["completed", "completed"])
    // The probe trips the key AFTER the main turn completes, so the wake
    // turn runs alone. `state::set` on the standalone worker fans out to the
    // delivery hop — the metadata-sidecar seam under test.
    .probe_after(
        1,
        "state::set",
        json!({ "scope": SCOPE, "key": KEY, "value": { "seq": 1 } }),
    )
    .function(record.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-arm",
                REGISTER,
                register_args,
                8,
                4,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-arm", "function_id": REGISTER }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-arm",
                                "is_error": false }),
                    ])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::text("armed", 10, 2)),
    )
    // The woken turn — seeded by the state worker's fire, running alone in
    // the SAME session. Binding retirement (max_fires spent) races the turn
    // start, so match the prompt loosely.
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(".")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::function_call(
                "call-record",
                &record,
                json!({ "value": "from-state-wake" }),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex(".")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::text("state wake recorded", 12, 3)),
    )
    .verify(|run| {
        // The second terminal turn exists ONLY if the state worker delivered
        // the metadata sidecar so the delivery hop could resolve its binding.
        run.expect_assistant_texts(["armed", "state wake recorded"])?;
        run.expect_function_calls("record", 1)?;
        run.expect_call_payload("record", json!({ "value": "from-state-wake" }))?;

        // The wake is a notification carrying the state event.
        let notification = run
            .transcript
            .iter()
            .filter_map(|item| {
                let msg = item.get("message")?;
                if msg.get("role").and_then(Value::as_str) != Some("user") {
                    return None;
                }
                let text: String = msg
                    .get("content")
                    .and_then(Value::as_array)?
                    .iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect();
                text.contains("[notification]").then_some(text)
            })
            .next();
        let notification = notification
            .ok_or_else(|| anyhow::anyhow!("no [notification] wake in the transcript"))?;
        anyhow::ensure!(
            notification.contains(SCOPE) && notification.contains(KEY),
            "the wake must carry the state event's watch: {notification}"
        );

        // The lifecycle bound retired the binding on its single delivery.
        let retired = run.transcript.iter().any(|item| {
            item.get("custom")
                .and_then(|c| c.get("custom_type"))
                .and_then(Value::as_str)
                == Some("trigger_fired")
                && item
                    .get("custom")
                    .and_then(|c| c.get("data"))
                    .and_then(|d| d.get("retired"))
                    .and_then(Value::as_bool)
                    == Some(true)
        });
        anyhow::ensure!(
            retired,
            "the max_fires bound must retire the binding on delivery"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_is_valid_and_awaits_the_wake_turn() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.probe_actions.len(), 1);
        assert_eq!(fixture.probe_actions[0].after_turns, 1);
    }
}
