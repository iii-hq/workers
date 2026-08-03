//! INT-020 — a narrowed child can always fetch contracts.
//!
//! The sub-agent contract makes an `engine::functions::info` round mandatory
//! before the first call, but parents narrow children to just the work
//! functions (`options.functions.allow: ["db::x"]`) — so the OBEDIENT child
//! was policy-denied its mandatory first step and reported
//! `FAILED: engine::functions::list/info is denied by policy`, while
//! siblings that skipped discovery succeeded. When compliance loses and
//! disobedience wins, the contract is broken: `child_functions` now unions
//! [`CHILD_DISCOVERY_ALLOW`] into every non-empty child allow-list.
//!
//! The grant is dispatch-level only: the engine's meta surface is not part of
//! the hydrated registry snapshot, so the child's NATIVE toolset stays exactly
//! its work functions (pinned below) — discovery is called by id, the way the
//! identity prompts teach it. The bite is generation 3: it requires the
//! child's `engine::functions::info` call to come back `is_error: false`,
//! which the pre-union harness answers with a policy denial.
//!
//! The child is spawned by the PROBE after the parent's turn completes — an
//! in-turn spawn interleaves the parent's post-spawn step with the child's
//! opening step in whichever order the scheduler lands them, and ordinal
//! dispatch cannot tolerate that race. The probe path exercises the same
//! `child_functions` union (the parentless arm) with a strictly sequential
//! generation order.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const SPAWN: &str = "harness::spawn";
const INFO: &str = "engine::functions::info";
const SUBAGENT_PROMPT: &str = "You are an iii sub-agent";

const TASK: &str = "Fetch the contract for {{run_id}}::record with engine::functions::info, \
                    then call it with value \"discovered\", then reply done.";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-020";
    const MESSAGE: &str = "Stand by while the probe dispatches the narrowed worker.";

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
        "child-discovery-granted",
        "A child narrowed to its work functions can still dispatch the mandatory contract \
         discovery pair; its native toolset stays the work functions only.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-020")
            .allow_function(&record),
    )
    // Only the parent is tracked; the child's recorder call is the whole-run
    // completion signal (the INT-015 pattern).
    .await_target_calls(1)
    .expect_traces(1)
    // A direct parentless spawn carrying the starving whitelist — work
    // function only, no discovery. A parentless spawn inherits nothing, so
    // model, provider, and the native exposure (the suite's pinned-toolset
    // surface; a requested policy defaults to agent_trigger) are explicit.
    .probe_after(
        1,
        SPAWN,
        json!({
            "session_id": "{{run_id}}-worker",
            "task": TASK,
            "model": "fixture-model",
            "provider": "scripted",
            "options": { "functions": {
                "allow": ["{{run_id}}::record"],
                "expose": "native"
            } },
        }),
    )
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
            .respond(Response::text("standing by", 8, 2)),
    )
    // The worker's opening step. Its toolset is EXACTLY the granted work
    // function — the discovery grant adds dispatch capability, never tools.
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(SUBAGENT_PROMPT)
                    .messages_subset([
                        json!({ "role": "user", "content": [{ "type": "text", "text": TASK }] }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-info",
                INFO,
                json!({ "function_id": "{{run_id}}::record" }),
                8,
                4,
            )),
    )
    // THE GATE: the mandatory discovery call must SUCCEED under the narrow
    // whitelist. The pre-union harness answers it with a policy denial
    // (is_error: true), this matcher never matches, and the run fails.
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex(SUBAGENT_PROMPT)
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-info", "function_id": INFO }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-info",
                                "is_error": false }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call(
                "call-record",
                &record,
                json!({ "value": "discovered" }),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(2)
                    .system_prompt_regex(SUBAGENT_PROMPT)
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-record" }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-record",
                                "is_error": false }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("done", 8, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["standing by"])?;
        // Reachable only through the successful discovery round: the child's
        // record call is the run's completion evidence.
        run.expect_target_calls(1)?;
        anyhow::ensure!(
            run.target_calls[0] == json!({ "value": "discovered" }),
            "recorder payload {:?} != discovered",
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
    fn fixture_gates_the_run_on_a_successful_discovery_dispatch() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.await_target_calls, Some(1));
        assert_eq!(fixture.script.generations.len(), 4);

        // The probe spawn carries the starving whitelist — work function
        // only, no discovery grant anywhere in options.
        assert_eq!(fixture.probe_actions.len(), 1);
        assert_eq!(fixture.probe_actions[0].function_id, SPAWN);
        assert_eq!(
            fixture.probe_actions[0]
                .payload
                .pointer("/options/functions/allow")
                .unwrap(),
            &json!(["{{run_id}}::record"])
        );

        // The gate requires the info call to SUCCEED.
        let gate = serde_json::to_string(&fixture.script.generations[2].match_.messages).unwrap();
        assert!(gate.contains("call-info"), "{gate}");
        assert!(gate.contains("\"is_error\":false"), "{gate}");
    }
}
