//! INT-023 — distinct model-provided function-call ids must retain distinct
//! durable results even when their punctuation used to sanitize identically.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const ID: &str = "INT-023";
const MESSAGE: &str = "Call the recorder twice with collision-control ids.";
const FINAL_TEXT: &str = "both collision-control results retained";

pub(super) fn scenario() -> ScenarioFixture {
    const FIRST_CALL_ID: &str = "call/a";
    const SECOND_CALL_ID: &str = "call a";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new(
        "{{run_id}}::record_collision",
        "Record one collision-control value.",
    )
    .request_schema(json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "value": { "type": "string" } },
        "required": ["value"]
    }))
    .returns_text("recorded");
    let first = json!({ "value": "first" });
    let second = json!({ "value": "second" });

    Scenario::new(
        ID,
        "function-call-id-collision",
        "Distinct function-call ids retain one durable result for each executed side effect.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-023")
            .allow_function(&record),
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
            .respond(Response::function_calls_raw(
                vec![
                    (FIRST_CALL_ID, record.id(), first.clone()),
                    (SECOND_CALL_ID, record.id(), second.clone()),
                ],
                16,
                8,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([
                        Message::user(MESSAGE),
                        Message::function_calls(
                            [
                                (FIRST_CALL_ID, &record, first.clone()),
                                (SECOND_CALL_ID, &record, second.clone()),
                            ],
                            &model,
                            16,
                            8,
                        ),
                        Message::function_result(FIRST_CALL_ID, &record, "recorded"),
                        Message::function_result(SECOND_CALL_ID, &record, "recorded"),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text(FINAL_TEXT, 24, 5)),
    )
    .verify(|run| {
        run.expect_assistant_texts([FINAL_TEXT])?;
        run.expect_message_counts(1, 2, 2)?;
        run.expect_function_calls("record_collision", 2)?;
        let payloads = run
            .calls("record_collision")
            .into_iter()
            .filter_map(|call| call.payload)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            payloads == [json!({ "value": "first" }), json!({ "value": "second" })],
            "controlled function payloads were {payloads:?}"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::script::JsonMatcherV1;

    #[test]
    fn requires_both_results_in_the_follow_up_history() {
        let fixture = scenario();
        let JsonMatcherV1::Exact { expected, .. } = &fixture.script.generations[1].match_.messages
        else {
            panic!("function history must be exact")
        };
        assert_eq!(expected.as_array().map(Vec::len), Some(4));
        assert_eq!(expected[2]["function_call_id"], "call/a");
        assert_eq!(expected[3]["function_call_id"], "call a");
    }
}
