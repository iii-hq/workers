//! INT-015 — a spawned child is a LEAF by capability, not by prompt: its
//! attempts to register a trigger and to spawn are refused by the dispatch
//! policy itself.
//!
//! The parent's own policy ALLOWS `engine::register_trigger` and
//! `harness::spawn` (it spawns the child with the first), and the child
//! inherits that allow — the denial comes purely from the CONTROL_PLANE deny
//! globs the spawn appends because `options.orchestrator` was not passed.
//! Both attempted calls come back `is_error: true`, the child's visible
//! toolset is just its granted recorder (`tools_exact` — the same globs
//! filter the native tool list), and the child still finishes its actual
//! assignment.
//!
//! The child session is untracked, so the recorder call is both the await
//! signal and the whole-run evidence (the INT-008 pattern).

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const SPAWN: &str = "harness::spawn";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-015";
    const MESSAGE: &str = "Spawn the probe child.";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new("{{run_id}}::record", "Record one value.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        }))
        .returns_text("recorded");

    // No `orchestrator`, no `options.functions` — the child inherits the
    // parent's FULL allow set and the leaf wall's deny globs on top.
    let spawn_args = json!({
        "task": "Probe your limits: attempt to register a trigger, attempt to spawn a \
                 sub-agent, then record what happened and stop.",
        "session_id": "{{run_id}}-leaf",
    });

    Scenario::new(
        ID,
        "leaf-denied-control-plane",
        "A spawned child without the orchestrator grant is policy-denied trigger registration \
         and spawning, and sees neither in its toolset.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-015")
            .allow_id(REGISTER)
            .allow_id(SPAWN)
            .allow_function(&record),
    )
    // Only the parent's turn is tracked; the child's completion signal is the
    // recorder call.
    .await_target_calls(1)
    .expect_traces(1)
    .function(record.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-spawn",
                SPAWN,
                spawn_args,
                8,
                4,
            )),
    )
    // The child's opening step arrives BEFORE the parent's post-spawn step
    // (its turn job is enqueued during the spawn dispatch, ahead of the
    // parent's re-enqueued next step). Its ONLY visible tool is the recorder:
    // the leaf deny globs filtered `harness::spawn` out of the native toolset
    // even though the inherited allow covers it — this tools_exact IS
    // acceptance criterion 4.
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex("You are an iii sub-agent")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-reg",
                REGISTER,
                json!({ "trigger_type": "state", "config": { "scope": "{{run_id}}-x" } }),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-spawn", "function_id": SPAWN }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-spawn",
                                "is_error": false }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("probe child spawned", 10, 2)),
    )
    // THE GATE, twice over: both control-plane attempts must come back as
    // policy errors. If the wall is missing, register SUCCEEDS (is_error
    // false — the intercept happily builds a binding), this matcher never
    // matches, and the run times out.
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex("You are an iii sub-agent")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-reg" }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-reg",
                                "is_error": true }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-nested",
                SPAWN,
                json!({ "task": "should never start" }),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(5)
            .expect(
                Request::new()
                    .turn_request_step(2)
                    .system_prompt_regex("You are an iii sub-agent")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-nested" }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-nested",
                                "is_error": true }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call(
                "call-record",
                &record,
                json!({ "value": "denied-as-designed" }),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(6)
            .expect(
                Request::new()
                    .turn_request_step(3)
                    .system_prompt_regex("You are an iii sub-agent")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("probe finished", 12, 3)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["probe child spawned"])?;
        // Whole-run probe evidence: the recorder call is reachable only after
        // BOTH denials matched (gen4/gen5 require is_error: true).
        run.expect_target_calls(1)?;
        anyhow::ensure!(
            run.target_calls[0] == json!({ "value": "denied-as-designed" }),
            "recorder payload {:?} != denied-as-designed",
            run.target_calls[0]
        );
        // And no denial leaked back into the parent as an injected message.
        for item in &run.transcript {
            let text = serde_json::to_string(item).unwrap_or_default();
            anyhow::ensure!(
                !text.contains("[child-failure]"),
                "a child outcome was injected into the parent: {text}"
            );
        }
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_awaits_the_leaf_through_its_recorder_call() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.await_target_calls, Some(1));
        assert_eq!(fixture.expected_traces(), 1);
        assert_eq!(fixture.script.generations.len(), 6);
    }
}
