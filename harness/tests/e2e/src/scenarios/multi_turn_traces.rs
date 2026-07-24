//! UI-002 — a function turn and a Console turn remain observable as distinct traces.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send, Tool,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

pub(super) const SECOND_MESSAGE: &str = "Return the second trace phrase.";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "UI-002";
    const FIRST_MESSAGE: &str = "Call the recorder once.";
    const CALL_ID: &str = "call-1";
    const FIRST_TEXT: &str = "recorded once";
    const SECOND_TEXT: &str = "second trace complete";

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
        "multi-turn-traces",
        "A function turn and a Console turn remain durable and observable as distinct traces.",
        ScenarioDriver::Playground,
        model.clone(),
    )
    .send(
        Send::message(FIRST_MESSAGE)
            .idempotency_key("{{run_id}}:ui-002")
            .allow_function(&record),
    )
    .function(record.clone())
    .terminal_turns(2)
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(FIRST_MESSAGE)])
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
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([
                        Message::user(FIRST_MESSAGE),
                        Message::function_call(CALL_ID, &record, arguments.clone(), &model, 8, 4),
                        Message::function_result(CALL_ID, &record, "recorded"),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text(FIRST_TEXT, 18, 2)),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex("agent_trigger")
                    .messages_exact([
                        Message::user(FIRST_MESSAGE),
                        Message::function_call(CALL_ID, &record, arguments.clone(), &model, 8, 4),
                        Message::function_result(CALL_ID, &record, "recorded"),
                        Message::assistant_text(FIRST_TEXT, &model, 18, 2),
                        Message::user(SECOND_MESSAGE),
                    ])
                    .tools_subset([Tool::named("agent_trigger")]),
            )
            .respond(Response::streamed_text(
                SECOND_TEXT,
                ["second trace ", "complete"],
                14,
                3,
            )),
    )
    .verify(|run| {
        run.expect_assistant_texts([FIRST_TEXT, SECOND_TEXT])?;
        run.expect_message_counts(2, 3, 1)?;
        run.expect_function_calls("record", 1)?;
        run.expect_call_payload("record", json!({ "value": "expected" }))?;
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_function_history_and_starts_the_console_turn_at_step_zero() {
        let fixture = scenario();
        let patterns: Vec<_> = fixture
            .script
            .generations
            .iter()
            .map(|generation| {
                let crate::types::script::JsonMatcherV1::Regex { pattern } =
                    &generation.match_.request_id
                else {
                    panic!("request id must be a regex")
                };
                pattern.as_str()
            })
            .collect();
        assert!(patterns[0].ends_with(":0$"), "{}", patterns[0]);
        assert!(patterns[1].ends_with(":1$"), "{}", patterns[1]);
        assert!(patterns[2].ends_with(":0$"), "{}", patterns[2]);
        assert_eq!(fixture.expected_terminal_turns, 2);
    }
}
