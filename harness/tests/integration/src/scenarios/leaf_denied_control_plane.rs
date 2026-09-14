//! INT-015 — a spawned child is a LEAF by capability, not by prompt. The wall
//! is drawn by SHAPE, not by function id: the child may arm a wake for its own
//! session, and may neither bind a mechanical call nor spawn.
//!
//! The parent's own policy ALLOWS `engine::register_trigger` and
//! `harness::spawn` (it spawns the child with the first), and the child
//! inherits that allow. `harness::spawn` is taken back by the CONTROL_PLANE
//! deny globs the spawn appends because `options.orchestrator` was not passed,
//! and is missing from the child's native toolset for the same reason.
//!
//! Registration is NOT taken back. Walling it off left a leaf with no way to
//! wait for anything — not its own background job, not its own compose
//! operation — so it polled or ran `shell::exec sleep` instead (MOT-4766). The
//! control-plane half of it is refused where it actually lives: a binding that
//! mechanically calls another function installs durable machinery the leaf does
//! not own and cannot unregister.
//!
//! So this scenario pins three outcomes in a row from one child: a self-wake
//! SUCCEEDS, a mechanical-call binding is refused, a spawn is refused — and the
//! child still finishes its actual assignment.
//!
//! The child session is untracked, so the recorder call is both the await
//! signal and the whole-run evidence (the INT-008 pattern).

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;
use crate::types::script::RouterDispatchV1;

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

    let mut fixture = Scenario::new(
        ID,
        "leaf-denied-control-plane",
        "A spawned child without the orchestrator grant may arm its own wake, and is refused \
         both a mechanical-call binding and a spawn.",
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
    // The child's opening step may race the parent's post-spawn step. Its
    // visible tools are the recorder plus the registration control: the leaf
    // deny globs filtered `harness::spawn` out of the native toolset even
    // though the inherited allow covers it, and left registration in — this
    // tools_exact IS acceptance criterion 4.
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex("You are an iii agent")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
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
    // GATE 1 — the half a leaf KEEPS. `call-reg` named no `function_id`, so it
    // is a wake for the child's own session and must come back clean. If the
    // wall is drawn by function id again, this comes back `is_error: true`,
    // the matcher never matches, and the run times out.
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex("You are an iii agent")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-reg" }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-reg",
                                "is_error": false }),
                    ])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-bind",
                REGISTER,
                json!({
                    "trigger_type": "state",
                    "config": { "scope": "{{run_id}}-y" },
                    "function_id": "{{run_id}}::record"
                }),
                8,
                4,
            )),
    )
    // GATE 2 — the half it does not. Naming a `function_id` asks for durable
    // machinery pointed at another function, which a leaf cannot unregister.
    .generation(
        Generation::new(5)
            .expect(
                Request::new()
                    .turn_request_step(2)
                    .system_prompt_regex("You are an iii agent")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-bind" }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-bind",
                                "is_error": true }),
                    ])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
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
        Generation::new(6)
            .expect(
                Request::new()
                    .turn_request_step(3)
                    .system_prompt_regex("You are an iii agent")
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
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::function_call(
                "call-record",
                &record,
                json!({ "value": "wake-armed-rest-denied" }),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(7)
            .expect(
                Request::new()
                    .turn_request_step(4)
                    .system_prompt_regex("You are an iii agent")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::text("probe finished", 12, 3)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["probe child spawned"])?;
        // Whole-run probe evidence: the recorder call is reachable only after
        // all three gates matched — the wake succeeded (gen4 requires
        // is_error: false) and both control-plane attempts were refused
        // (gen5/gen6 require is_error: true).
        run.expect_target_calls(1)?;
        anyhow::ensure!(
            run.target_calls[0] == json!({ "value": "wake-armed-rest-denied" }),
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
    .build();
    // Spawn schedules the parent continuation and child independently.
    fixture.script.dispatch = RouterDispatchV1::MatchAny;
    fixture
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
        // 7: two parent steps, and five from the child — open, wake (allowed),
        // mechanical bind (refused), spawn (refused), record.
        assert_eq!(fixture.script.generations.len(), 7);
        assert_eq!(fixture.script.dispatch, RouterDispatchV1::MatchAny);
    }
}
