//! E2E-005 — a reaction registered with no `options` inherits the registering
//! turn's dispatch policy.
//!
//! The rctest-k7m3 run-3 failure shape: an agent registers a wrap-up reaction
//! (no `options.functions`) explicitly delivered back into its own session;
//! the reaction used to fall back to the read-only baseline and was denied the
//! functions every earlier turn in that chat could call. With the
//! registrant-policy stamp the reaction inherits the registering turn's
//! policy, so its scripted call to the controlled function must dispatch
//! successfully — before the fix that call errors with "not permitted by this
//! agent's dispatch policy".
//!
//! The coalescing half of the same PR is pinned at the unit level
//! (`fire_gate_coalesces_capped_fires_newest_wins`): its trailing fire lands
//! when the 60s window frees, which cannot fit the e2e deadline.

use serde_json::{json, Value};

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::evidence_data::message_text;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "E2E-005";
    const MESSAGE: &str = "Arm the wrap-up reaction.";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new(
        "{{run_id}}::record",
        "Record one integration fixture value.",
    )
    .request_schema(json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "value": { "type": "string" } },
        "required": ["value"]
    }))
    .returns_text("recorded");

    Scenario::new(
        ID,
        "reaction-policy-inheritance",
        "A reaction with no options inherits the registering turn's dispatch policy.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:e2e-005")
            .allow_id(REGISTER)
            .allow_function(&record),
    )
    // Turn 1 completes, the turn-completed binding fires, and the explicitly
    // pinned reaction seeds a SECOND terminal turn in the same session.
    .terminal_turn_statuses(["completed", "completed"])
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
                json!({
                    "trigger_type": "harness::turn-completed",
                    "config": { "session_id": "{{session_id}}" },
                    "function_id": "harness::react",
                    "metadata": {
                        // Deliberately NO `options`: the reaction must inherit
                        // the registering turn's policy to call the recorder.
                        // Pin the delivery session explicitly; unpinned
                        // reactions intentionally create fresh children.
                        "session_id": "{{session_id}}",
                        "task": "The wrap-up fired. Call the recorder exactly once \
                                 with value \"from-reaction\", then stop."
                    }
                }),
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
    // The reaction turn (new turn, steps restart at 0). Its tools carry the
    // controlled function ONLY because the policy was inherited — the
    // read-only baseline exposes no worker functions at all.
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    // Reaction turns run the sub-agent prompt, not the default.
                    .system_prompt_regex(".")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result", "function_call_id": "call-arm" }),
                        json!({ "role": "assistant" }),
                        // The reaction seed, fenced with the fired event.
                        json!({ "role": "user" }),
                    ])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::function_call(
                "call-record",
                &record,
                json!({ "value": "from-reaction" }),
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
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-record" }
                        ] }),
                        // The dispatch the whole scenario gates on: succeeds
                        // only under the inherited policy.
                        json!({ "role": "function_result", "function_call_id": "call-record",
                                "is_error": false }),
                    ])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::text("reaction recorded", 12, 3)),
    )
    .function(record.clone())
    .verify(|run| {
        run.expect_assistant_texts(["armed", "reaction recorded"])?;
        run.expect_function_calls("record", 1)?;
        run.expect_call_payload("record", json!({ "value": "from-reaction" }))?;

        let regs = run.function_results(REGISTER);
        anyhow::ensure!(
            regs.len() == 1 && regs[0].get("is_error").and_then(Value::as_bool) == Some(false),
            "the reaction registration must succeed: {regs:?}"
        );

        // The core claim: nothing in the whole run was policy-denied. Before
        // the registrant-policy stamp, the reaction turn's recorder call died
        // here with "not permitted by this agent's dispatch policy".
        for item in &run.transcript {
            let msg = item.get("message").cloned().unwrap_or(Value::Null);
            if msg.get("role").and_then(Value::as_str) == Some("function_result") {
                let text = message_text(&msg);
                anyhow::ensure!(
                    !text.contains("not permitted"),
                    "a dispatch was policy-denied — the reaction did not inherit \
                     the registrant's policy: {text}"
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
    fn fixture_is_valid_and_expects_two_terminal_turns() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.await_target_calls, None);
        assert_eq!(fixture.expected_traces(), 1);
    }
}
