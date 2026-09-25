//! INT-036 — every router request of a session extends the one before it
//! (MOT-4845). Opus 5.5 and Fable 5.1 bind each signed thinking block to the
//! exact prefix that produced it — system prompt, tools and every earlier
//! message — so a step that rewrites what an earlier step already sent is a
//! history edit: a 400 on new accounts, silently dropped reasoning otherwise.
//!
//! Turn 1 makes two function calls and receives a steer queued during the
//! second. Turn 2 arrives through a probe send that moves the working
//! directory (the runtime aid frozen into the system prompt), makes one more
//! call and answers. The queue drain and the runtime-context notice must
//! reach the model as appended messages, never as rewrites.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const ID: &str = "INT-036";
const MESSAGE: &str = "Record two fixture values.";
const STEER: &str = "Also say when both are recorded.";
const FOLLOW_UP: &str = "Record one more value from the new directory.";

pub(super) fn scenario() -> ScenarioFixture {
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
    let call = |id: &str, value: &str| {
        Response::function_call(id, &record, json!({ "value": value }), 10, 4)
    };
    let turn_one = |step: u64| {
        Request::new()
            .turn_request_step(step)
            .system_prompt_sha256("{{system_prompt_sha256}}")
            .messages_subset([Message::user(MESSAGE)])
            .tools_exact([record.tool()])
    };
    // A harness that rewrites the runtime aid changes turn 2's prompt; match it
    // permissively so such a run reaches `verify`, which names the edit.
    let turn_two = |step: u64| {
        Request::new()
            .turn_request_step(step)
            .system_prompt_regex("(?s).+")
            .messages_subset([
                Message::user(MESSAGE),
                json!({ "role": "assistant" }),
                json!({ "role": "function_result", "function_call_id": "call-1" }),
                json!({ "role": "assistant" }),
                json!({ "role": "function_result", "function_call_id": "call-2" }),
                Message::user(STEER),
                json!({ "role": "assistant" }),
                Message::user(FOLLOW_UP),
            ])
            .tools_exact([record.tool()])
    };

    Scenario::new(
        ID,
        "binding-prefix-append-only",
        "Across two function calls, a queued steer, and a second turn that moves the working directory, every router request of the session extends the previous one: system prompt, sections, tools, and all earlier messages unchanged.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-036")
            .allow_function(&record),
    )
    .function(record.clone())
    .terminal_turns(2)
    .probe_after(
        1,
        "harness::send",
        json!({
            "session_id": "{{session_id}}",
            "message": FOLLOW_UP,
            "idempotency_key": "{{run_id}}:integration-036-b",
            "options": {
                "functions": { "allow": ["{{run_id}}::record"], "expose": "native" },
                "metadata": { "fs_scope": { "root": "/tmp/int-036" } }
            }
        }),
    )
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact([record.tool()]),
            )
            .respond(call("call-1", "first")),
    )
    .generation(
        Generation::new(2)
            .expect(turn_one(1))
            .parked_message(STEER)
            .respond(call("call-2", "second")),
    )
    .generation(
        Generation::new(3)
            .expect(turn_one(2).messages_subset([
                Message::user(MESSAGE),
                json!({ "role": "assistant" }),
                json!({ "role": "function_result", "function_call_id": "call-1" }),
                json!({ "role": "assistant" }),
                json!({ "role": "function_result", "function_call_id": "call-2" }),
                Message::user(STEER),
            ]))
            .respond(Response::text("recorded both", 30, 2)),
    )
    .generation(
        Generation::new(4)
            .expect(turn_two(0))
            .respond(call("call-3", "third")),
    )
    .generation(
        Generation::new(5)
            .expect(turn_two(1))
            .respond(Response::text("recorded the third", 40, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["recorded both", "recorded the third"])?;
        run.expect_function_calls("record", 3)?;
        run.expect_no_duplicate_messages()?;
        run.expect_append_only_requests()
    })
    .build()
}
