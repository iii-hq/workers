//! INT-018 — an in-turn spawn that names an EXISTING session is confined to
//! the caller's own tree.
//!
//! Models are not RNGs: two runs of the same prompt re-invent the same
//! "random" child session id, and without the reuse guard the second run's
//! spawn silently appended its task to the first run's child — old transcript
//! carried over, console still nested under the original parent. The fixture
//! drives both halves of the contract through the public path:
//!
//!   1. plant `{{run_id}}-taken` owned by `{{run_id}}-other-run` with a plain
//!      `session::ensure` call — the prior owner never has to be an agent;
//!   2. spawn into it → `is_error: true` naming the owner (the gen-3 matcher;
//!      silent reuse would answer `is_error: false` and never match), and no
//!      turn ever starts in the taken session — a hijack turn would reach the
//!      scripted router as an unmatched generation and fail the run;
//!   3. spawn `{{run_id}}-fresh` → created, reported `reused: false`;
//!   4. spawn `{{run_id}}-fresh` AGAIN → own-child reuse is allowed, reported
//!      with the reuse note + `reused: true`, and the second task arrives as
//!      a new turn on top of the child's retained transcript (the gen-6
//!      matcher pins task one, its reply, and task two in one history).
//!
//! The child session is untracked; its recorder call is the whole-run
//! completion signal (the INT-015 pattern).

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const ENSURE: &str = "session::ensure";
const SPAWN: &str = "harness::spawn";
const SUBAGENT_PROMPT: &str = "You are an iii sub-agent";

const TASK_ONE: &str = "First assignment: reply done.";
const TASK_TWO: &str = "Second assignment: record the value retasked, then reply done.";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-018";
    const MESSAGE: &str = "Plant a taken session, then exercise spawn reuse.";

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
        "spawn-reuse-guard",
        "An in-turn spawn into another owner's existing session is refused naming the owner; \
         re-spawning its own child is allowed and reported as reuse.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-018")
            // Sorted: the harness renders the policy prompt line in id order,
            // and the dsl template joins this list verbatim.
            .allow_id(SPAWN)
            .allow_function(&record)
            .allow_id(ENSURE),
    )
    // Only the parent's turn is tracked; the child's recorder call is the
    // whole-run completion signal for its second (re-tasked) turn.
    .await_target_calls(1)
    .expect_traces(1)
    .function(record.clone())
    // Parent step 0: plant the foreign-owned session. `session::ensure` is an
    // ordinary registry function — this is exactly how a colliding id comes to
    // exist in the wild (an earlier run created it), minus the second agent.
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
                "call-plant",
                ENSURE,
                json!({
                    "session_id": "{{run_id}}-taken",
                    "metadata": { "parent_session_id": "{{run_id}}-other-run" }
                }),
                8,
                4,
            )),
    )
    // Parent step 1: the collision — spawn into the taken session.
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-plant", "function_id": ENSURE }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-plant",
                                "is_error": false }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-collide",
                SPAWN,
                json!({
                    "session_id": "{{run_id}}-taken",
                    "task": "hijack attempt — this task must never start a turn"
                }),
                8,
                4,
            )),
    )
    // THE GATE: the collision must come back as an error. Before the guard,
    // this spawn succeeded silently (is_error: false) — this matcher would
    // never match — and the hijack turn's router request had no generation to
    // serve it.
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
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-collide" }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-collide",
                                "is_error": true }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-fresh",
                SPAWN,
                json!({ "session_id": "{{run_id}}-fresh", "task": TASK_ONE }),
                8,
                4,
            )),
    )
    // The fresh child's opening step arrives BEFORE the parent's post-spawn
    // step (its turn job is enqueued during the spawn dispatch, ahead of the
    // parent's re-enqueued next step).
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(SUBAGENT_PROMPT)
                    .messages_subset([
                        json!({ "role": "user", "content": [{ "type": "text", "text": TASK_ONE }] }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("first assignment done", 10, 2)),
    )
    // Parent step 3: re-task the OWN child by its explicit id — the allowed
    // reuse (retry/re-task flows must keep working).
    .generation(
        Generation::new(5)
            .expect(
                Request::new()
                    .turn_request_step(3)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-fresh" }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-fresh",
                                "is_error": false }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-retask",
                SPAWN,
                json!({ "session_id": "{{run_id}}-fresh", "task": TASK_TWO }),
                8,
                4,
            )),
    )
    // The reuse semantics made visible: the child's SECOND turn assembles the
    // retained transcript — task one, its reply, then task two appended.
    .generation(
        Generation::new(6)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(SUBAGENT_PROMPT)
                    .messages_subset([
                        json!({ "role": "user", "content": [{ "type": "text", "text": TASK_ONE }] }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user", "content": [{ "type": "text", "text": TASK_TWO }] }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::function_call(
                "call-record",
                &record,
                json!({ "value": "retasked" }),
                8,
                4,
            )),
    )
    // Parent step 4: the re-task settled without error; the parent finishes.
    .generation(
        Generation::new(7)
            .expect(
                Request::new()
                    .turn_request_step(4)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-retask" }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-retask",
                                "is_error": false }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("collision refused; own child re-tasked", 16, 3)),
    )
    .generation(
        Generation::new(8)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex(SUBAGENT_PROMPT)
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-record" }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-record",
                                "is_error": false }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("retask done", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["collision refused; own child re-tasked"])?;
        run.expect_target_calls(1)?;
        anyhow::ensure!(
            run.target_calls[0] == json!({ "value": "retasked" }),
            "recorder payload {:?} != retasked",
            run.target_calls[0]
        );
        let transcript = serde_json::to_string(&run.transcript).unwrap_or_default();
        // The refusal is diagnosable from the parent transcript alone: it
        // names the taken id, the owner, and the remedy.
        for needle in [
            "already exists and belongs to parent",
            "-taken",
            "-other-run",
            "omit session_id",
        ] {
            anyhow::ensure!(
                transcript.contains(needle),
                "refusal evidence missing {needle:?} in the parent transcript"
            );
        }
        // Reuse is reported exactly once — on the re-task, never on the
        // fresh spawn.
        anyhow::ensure!(
            transcript.matches("already existed").count() == 1,
            "expected exactly one reuse note in the parent transcript"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_pins_the_refusal_and_the_own_child_reuse() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.await_target_calls, Some(1));
        assert_eq!(fixture.expected_traces(), 1);
        assert_eq!(fixture.script.generations.len(), 8);

        // The collision gate is load-bearing: silent reuse answers
        // `is_error: false` and can never satisfy generation 3.
        let gate = serde_json::to_string(&fixture.script.generations[2].match_.messages).unwrap();
        assert!(gate.contains("call-collide"), "{gate}");
        assert!(gate.contains("\"is_error\":true"), "{gate}");

        // Both spawns of the child name the same explicit session id.
        let script = serde_json::to_string(&fixture.script.generations).unwrap();
        assert_eq!(script.matches("{{run_id}}-fresh").count(), 2, "{script}");
    }
}
