//! E2E-006 — a reaction bound to a `state` key fires through the standalone
//! state worker, carrying the registration-metadata sidecar.
//!
//! This is the cross-worker seam every rctest run turned on (MOT-4209): the
//! state worker's fan-out must invoke `harness::react` WITH the binding's
//! metadata, or the reaction silently no-ops. This run's stack includes the
//! real `state` worker (see `stack/config.rs`); the engine builtin is
//! disabled, so the standalone worker owns the `state` trigger type exactly
//! as in production.
//!
//! Determinism comes from the probe hook: the main turn only REGISTERS the
//! state-key reaction and completes. The PROBE then writes the key (after the
//! first terminal turn), so the reaction fires while the session is idle — its
//! turn is the only one active and the scripted, strict-ordinal router matches
//! it cleanly. The reaction's recorder call is the proof the sidecar arrived;
//! drop it and no second turn is ever seeded and the run times out.

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
    const ID: &str = "E2E-006";
    const MESSAGE: &str = "Arm a state-key reaction.";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new("{{run_id}}::record", "Record one value.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        }))
        .returns_text("recorded");

    // No `options`: the reaction inherits the registering turn's policy
    // (MOT-4212), which allows the recorder.
    let register_args = json!({
        "trigger_type": "state",
        "config": { "scope": SCOPE, "key": KEY },
        "function_id": "harness::react",
        "metadata": {
            "task": "The state key changed. Call the recorder exactly once with \
                     value \"from-state-reaction\", then stop."
        }
    });

    Scenario::new(
        ID,
        "state-worker-sidecar",
        "A state-key reaction fires through the standalone state worker with its metadata sidecar.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:e2e-006")
            .allow_id(REGISTER)
            .allow_function(&record),
    )
    // Main turn (1) + the state-fired reaction turn (2).
    .terminal_turn_statuses(["completed", "completed"])
    // The probe trips the key AFTER the main turn completes, so the reaction
    // turn runs alone. `state::set` on the standalone worker fans out to the
    // react binding — the seam under test.
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
                    .tools_exact([record.tool()]),
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
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("armed", 10, 2)),
    )
    // The reaction turn — seeded by the state worker's fire, running alone.
    // Reaction turns run the sub-agent prompt, so match it by presence.
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(".")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call(
                "call-record",
                &record,
                json!({ "value": "from-state-reaction" }),
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
                    // The reaction runs in the registering session, so its
                    // history begins with the original user message; the
                    // strict-ordinal router + step already sequence gen3→gen4.
                    // The recorder call itself is asserted in `verify`.
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("state reaction recorded", 12, 3)),
    )
    .verify(|run| {
        // The second terminal turn exists ONLY if the state worker delivered
        // the metadata sidecar so harness::react could resolve the task.
        run.expect_assistant_texts(["armed", "state reaction recorded"])?;
        run.expect_function_calls("record", 1)?;
        run.expect_call_payload("record", json!({ "value": "from-state-reaction" }))?;

        for item in &run.transcript {
            let msg = item.get("message").cloned().unwrap_or(Value::Null);
            if msg.get("role").and_then(Value::as_str) == Some("function_result") {
                let text: String = msg
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|blocks| {
                        blocks
                            .iter()
                            .filter_map(|b| b.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .concat()
                    })
                    .unwrap_or_default();
                anyhow::ensure!(
                    !text.contains("not permitted"),
                    "a dispatch was denied: {text}"
                );
            }
        }
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_is_valid_and_awaits_the_reaction_turn() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.probe_actions.len(), 1);
        assert_eq!(fixture.probe_actions[0].after_turns, 1);
    }
}
