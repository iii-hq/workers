//! INT-011 — a deny-only policy rejects its target before invocation.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send, Tool,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-011";
    const MESSAGE: &str = "Attempt the denied recorder call.";
    const CALL_ID: &str = "call-denied";

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
    .returns_text("must not execute");
    let arguments = json!({ "value": "blocked" });

    Scenario::new(
        ID,
        "deny-only-function",
        "A deny-only dispatch policy rejects a matching function before invocation.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-011")
            .deny_function(&record),
    )
    .function(record.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_subset([Tool::named("agent_trigger")]),
            )
            .respond(Response::agent_trigger_call(
                CALL_ID,
                &record,
                arguments.clone(),
                9,
                4,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([
                        Message::user(MESSAGE),
                        Message::agent_trigger_call(CALL_ID, &record, arguments, &model, 9, 4),
                        Message::policy_denied_result(CALL_ID, &record),
                    ])
                    .tools_subset([Tool::named("agent_trigger")]),
            )
            .respond(Response::text("denial observed", 15, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["denial observed"])?;
        run.expect_message_counts(1, 2, 1)?;
        run.expect_function_calls("record", 0)?;
        run.expect_no_duplicate_messages()
    })
    .build()
}
