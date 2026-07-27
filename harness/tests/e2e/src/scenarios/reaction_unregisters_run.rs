//! E2E-008 — a reaction session unregisters its registrant's subscription.
//!
//! This gates the lineage-unregister seam (MOT-4217): unregistration is
//! owner-session-scoped, and before the fix a reaction running in a DIFFERENT
//! session than its registrant got `subscription belongs to a different
//! session` — the rctest5 cleanup deadlock (the registrant parked on a wake
//! its children could no longer satisfy, so nobody could tear the run down).
//! With lineage recorded at spawn, the reaction session's chain reaches the
//! owner and the unregister succeeds.
//!
//! The subscription id is RUNTIME-generated (`sub_<uuid>`), which a static
//! fixture cannot know — this is the scenario the router's serve-time capture
//! exists for: gen2 captures the id out of the registration function_result
//! in its own matched request, and the cleaner turn's unregister call echoes
//! it via `[[cap:subid]]`.
//!
//! The cleaner turn runs in a PINNED separate session (that's the point), so
//! neither turn completions nor session traces witness it; the recorder call
//! (probe-side whole-run evidence) is the completion signal — and it is only
//! reachable through gen4's matcher, which requires the unregister
//! function_result with `is_error: false`. Without the lineage fix the
//! unregister errors, gen4 never matches, and the run times out.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const UNREGISTER: &str = "engine::unregister_trigger";
const SCOPE: &str = "e2e-008";
const KEY: &str = "go";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "E2E-008";
    const MESSAGE: &str = "Arm a self-cleaning reaction.";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new("{{run_id}}::record", "Record one value.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        }))
        .returns_text("recorded");

    // Task reaction pinned to a DIFFERENT session — the cleaner. It inherits
    // the registering turn's policy (register/unregister/record allowed).
    // `once: false` is load-bearing: a one-shot binding retires itself after
    // its first spawn, and unregistering an already-gone id trivially
    // succeeds (`removed: false`) without ever exercising the ownership
    // check. The STANDING binding is still live when the cleaner calls
    // unregister, so the call must pass the lineage grant to remove it.
    let register_args = json!({
        "trigger_type": "state",
        "config": { "scope": SCOPE, "key": KEY },
        "function_id": "harness::react",
        "once": false,
        "metadata": {
            "task": "You are the cleanup agent. Unregister this run's subscription, then \
                     call the recorder exactly once with value \"cleaned\", then stop.",
            "session_id": "{{run_id}}-cleaner"
        }
    });

    Scenario::new(
        ID,
        "reaction-unregisters-run",
        "A reaction session in the registrant's lineage unregisters the registrant's subscription.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:e2e-008")
            .allow_id(REGISTER)
            .allow_id(UNREGISTER)
            .allow_function(&record),
    )
    // The cleaner session is untracked: its turns don't complete on the
    // tracked session and its trace groups under its own session id. The
    // recorder call is therefore both the await signal and the evidence.
    .await_target_calls(1)
    .expect_traces(1)
    .probe_after(
        1,
        "state::set",
        json!({ "scope": SCOPE, "key": KEY, "value": { "go": true } }),
    )
    .function(record.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact_after_controls([REGISTER, UNREGISTER], [record.tool()]),
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
                    .tools_exact_after_controls([REGISTER, UNREGISTER], [record.tool()]),
            )
            // The registration function_result in THIS request carries the
            // runtime subscription id; capture it for the cleaner's call.
            .capture("subid", "(sub_[0-9a-f]{32})")
            .respond(Response::text("armed", 10, 2)),
    )
    // The cleaner turn — spawned by the probe-tripped fire into its own
    // pinned session.
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(".")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact_after_controls([REGISTER, UNREGISTER], [record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-unreg",
                UNREGISTER,
                json!({ "id": "[[cap:subid]]" }),
                8,
                4,
            )),
    )
    // THE GATE: this matcher demands the unregister result with
    // `is_error: false` AND `removed: true` — the live standing binding was
    // actually torn down. Pre-MOT-4217 the harness answers `subscription
    // belongs to a different session` (an error result), this never matches,
    // and the run times out before any recorder call.
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex(".")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-unreg", "function_id": UNREGISTER }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-unreg",
                                "is_error": false,
                                "content": [{ "type": "text", "text": "{\"removed\":true}" }] }),
                    ])
                    .tools_exact_after_controls([REGISTER, UNREGISTER], [record.tool()]),
            )
            .respond(Response::function_call(
                "call-record",
                &record,
                json!({ "value": "cleaned" }),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(5)
            .expect(
                Request::new()
                    .turn_request_step(2)
                    .system_prompt_regex(".")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact_after_controls([REGISTER, UNREGISTER], [record.tool()]),
            )
            .respond(Response::text("cleaned up", 12, 3)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["armed"])?;
        // Whole-run probe evidence: exactly one recorder call, reachable only
        // after the successful (is_error: false) unregister matched gen4.
        run.expect_target_calls(1)?;
        anyhow::ensure!(
            run.target_calls[0] == json!({ "value": "cleaned" }),
            "recorder payload {:?} != cleaned",
            run.target_calls[0]
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_captures_the_subscription_id_and_awaits_the_cleaner() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.await_target_calls, Some(1));
        assert_eq!(fixture.expected_traces(), 1);
        let gen2 = fixture
            .script
            .generations
            .iter()
            .find(|g| g.ordinal == 2)
            .unwrap();
        assert_eq!(gen2.captures.len(), 1);
        assert_eq!(gen2.captures[0].name, "subid");
    }
}
