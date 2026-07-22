//! E2E-002 — a controlled function runs exactly once and closes the turn.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "E2E-002";
    const MESSAGE: &str = "Call the recorder once.";
    const CALL_ID: &str = "call-1";

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
    let arguments = json!({ "value": "expected" });

    Scenario::new(
        ID,
        "exactly-once-function",
        "The controlled function runs exactly once.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:e2e-002")
            .allow_function(&record),
    )
    .function(record.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call(
                CALL_ID,
                &record,
                arguments.clone(),
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
                    .messages_exact([
                        Message::user(MESSAGE),
                        Message::function_call(CALL_ID, &record, arguments.clone(), &model),
                        Message::function_result(CALL_ID, &record, "recorded"),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("recorded once", 18, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["recorded once"])?;
        run.expect_function_calls("record", 1)?;
        run.expect_call_payload("record", json!({ "value": "expected" }))?;
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::script::JsonMatcherV1;

    #[test]
    fn pins_function_call_and_result_history() {
        let fixture = scenario();
        let JsonMatcherV1::Exact { expected, .. } = &fixture.script.generations[1].match_.messages
        else {
            panic!("function history must be exact")
        };
        assert_eq!(expected.as_array().unwrap().len(), 3);
        assert_eq!(expected[1]["content"][0]["id"], "call-1");
        assert_eq!(expected[2]["role"], "function_result");
    }
}
