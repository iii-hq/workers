//! INT-010 — the shipped policy executes a function when `options` is omitted.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send, Tool,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-010";
    const MESSAGE: &str = "Call the recorder through the default dispatch policy.";
    const CALL_ID: &str = "call-default";

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
    let arguments = json!({ "value": "default-policy" });

    Scenario::new(
        ID,
        "default-policy-function",
        "The shipped dispatch policy executes a function when send options are omitted.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-010")
            .with_shipped_default(),
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
                10,
                5,
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
                        Message::agent_trigger_call(
                            CALL_ID,
                            &record,
                            arguments.clone(),
                            &model,
                            10,
                            5,
                        ),
                        Message::function_result(CALL_ID, &record, "recorded"),
                    ])
                    .tools_subset([Tool::named("agent_trigger")]),
            )
            .respond(Response::text("default policy executed once", 18, 3)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["default policy executed once"])?;
        run.expect_function_calls("record", 1)?;
        run.expect_call_payload("record", json!({ "value": "default-policy" }))?;
        run.expect_no_duplicate_messages()
    })
    .build()
}
