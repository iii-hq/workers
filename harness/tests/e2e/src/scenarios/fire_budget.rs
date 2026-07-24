//! E2E-010 — a `max_fires` budget retires a standing binding after its Nth
//! delivered fire.
//!
//! This gates the platform's termination guarantee for recurring bindings
//! (rctest7 postmortem): a 10-second gate-check cron polled FOREVER when its
//! gate condition became unreachable — at 6 fires/min the fire-rate breaker
//! correctly never engages, so nothing bounded the loop except the operator.
//! `metadata.max_fires` is that bound: the final budgeted fire is delivered
//! normally, stamped `__final_fire`, and the binding then retires through the
//! `once` teardown path.
//!
//! Choreography (all deterministic, no sleeps):
//! 1. Turn 1 registers a STANDING (`once: false`) call-mode reaction on a
//!    state key with `max_fires: 2`; gen2 captures the runtime subscription
//!    id for turn 2.
//! 2. The probe writes the key twice (each write gated on the previous
//!    recorder call) — fires 1 and 2 dispatch the recorder, fire 2 carrying
//!    the `__final_fire` budget stamps and triggering self-retirement.
//! 3. Only after both calls does the probe steer turn 2, which tries to
//!    unregister the captured id. The matcher demands `removed: false` — the
//!    budget already tore the binding down, locally AND engine-side (a
//!    retained local mapping would answer `removed: true` and never match).
//! 4. With retirement proven, the probe writes the key a third time: the
//!    binding is gone at the engine, so no third recorder call can exist —
//!    verified by the whole-run call count.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const UNREGISTER: &str = "engine::unregister_trigger";
const SCOPE: &str = "e2e-010";
const KEY: &str = "tick";
const MESSAGE_2: &str = "The budget is spent. Confirm the binding retired itself.";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "E2E-010";
    const MESSAGE: &str = "Arm a budgeted recurring reaction.";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new("{{run_id}}::record", "Record one event.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": true
        }))
        .returns_text("recorded");

    // Standing call-mode reaction with a lifetime budget of 2 fires.
    // `once: false` is load-bearing: state edges default to one-shot, and a
    // one-shot binding would retire after fire 1 without ever exercising the
    // budget path.
    let register_args = json!({
        "trigger_type": "state",
        "config": { "scope": SCOPE, "key": KEY },
        "function_id": "harness::react",
        "once": false,
        "metadata": {
            "call": { "function_id": "{{run_id}}::record" },
            "max_fires": 2
        }
    });

    Scenario::new(
        ID,
        "fire-budget-retires-binding",
        "A standing reaction with max_fires delivers exactly N fires and then unregisters itself.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:e2e-010")
            .allow_id(REGISTER)
            .allow_id(UNREGISTER)
            .allow_function(&record),
    )
    // Turn 1 (arm) + turn 2 (the probe-steered retirement check).
    .terminal_turn_statuses(["completed", "completed"])
    // Call-mode fires seed no session turns and no traces: only Send's own
    // trace and the probe-steered second send trace exist.
    .expect_traces(2)
    .await_target_calls(2)
    .probe_after(
        1,
        "state::set",
        json!({ "scope": SCOPE, "key": KEY, "value": { "n": 1 } }),
    )
    // Fire 2 only after fire 1's recorder call is evidence — budgeted fires
    // must not race each other or the count assertion below is ambiguous.
    .probe_after_calls(
        1,
        1,
        "state::set",
        json!({ "scope": SCOPE, "key": KEY, "value": { "n": 2 } }),
    )
    // Turn 2 fires only after BOTH budgeted calls landed.
    .probe_after_calls(
        1,
        2,
        "harness::send",
        json!({ "session_id": "{{session_id}}", "message": MESSAGE_2 }),
    )
    // After turn 2 proved retirement, a third write must fire NOTHING: the
    // engine binding is gone. The whole-run call count is the witness.
    .probe_after(
        2,
        "state::set",
        json!({ "scope": SCOPE, "key": KEY, "value": { "n": 3 } }),
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
                    .turn_request_step(1)
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
            // The registration function_result in THIS request carries the
            // runtime subscription id; turn 2's unregister echoes it.
            .capture("subid", "(sub_[0-9a-f]{32})")
            .respond(Response::text("armed", 10, 2)),
    )
    // Turn 2, steered strictly after both budgeted fires delivered.
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
                        json!({ "role": "user", "content": [
                            { "type": "text", "text": MESSAGE_2 }
                        ] }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-unreg",
                UNREGISTER,
                json!({ "id": "[[cap:subid]]" }),
                8,
                4,
            )),
    )
    // THE GATE: `removed: false` — the budget already unregistered the
    // binding (a live local mapping would answer `removed: true`, this never
    // matches, and the run times out).
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-unreg", "function_id": UNREGISTER }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-unreg",
                                "is_error": false,
                                "content": [{ "type": "text", "text": "{\"removed\":false}" }] }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("budget confirmed", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["armed", "budget confirmed"])?;
        // Exactly two fires, ever — the third write happened strictly after
        // retirement was proven, so a third call means the budget leaked.
        run.expect_target_calls(2)?;
        let seq = |i: usize, ptr: &str| run.target_calls[i].pointer(&format!("/event/{ptr}")).cloned();
        anyhow::ensure!(
            seq(0, "__fire_seq") == Some(json!(1))
                && seq(0, "__fire_budget") == Some(json!(2))
                && seq(0, "__final_fire").is_none(),
            "call #1 must be budgeted fire 1 of 2 and not final: {:?}",
            run.target_calls[0]
        );
        anyhow::ensure!(
            seq(1, "__fire_seq") == Some(json!(2)) && seq(1, "__final_fire") == Some(json!(true)),
            "call #2 must be the stamped final fire: {:?}",
            run.target_calls[1]
        );
        let note = seq(1, "__fire_budget_note")
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        anyhow::ensure!(
            note.contains("exhausted") && note.contains("no further fires"),
            "final fire must explain the exhausted budget: {note:?}"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_gates_each_fire_and_the_retirement_check() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.await_target_calls, Some(2));
        assert_eq!(fixture.expected_traces(), 2);
        assert_eq!(fixture.probe_actions.len(), 4);
        assert_eq!(fixture.probe_actions[1].after_target_calls, Some(1));
        assert_eq!(fixture.probe_actions[2].after_target_calls, Some(2));
        assert_eq!(fixture.probe_actions[3].after_turns, 2);
    }
}
