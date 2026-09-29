//! INT-032 — a call that already failed twice with the identical error is
//! answered locally: the third identical call never reaches the target.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-032";
    const MESSAGE: &str = "Call the failing recorder until it works.";
    const ERROR: &str = "recorder unavailable";

    let model = Model::scripted("fixture-model");
    let fail = ControlledFunction::new(
        "{{run_id}}::fail",
        "Record one integration fixture value; always fails.",
    )
    .request_schema(json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "value": { "type": "string" } },
        "required": ["value"]
    }))
    .returns_error(ERROR);
    let arguments = json!({ "value": "same" });
    let failed = |call_id: &str| {
        json!({ "role": "function_result", "function_call_id": call_id, "is_error": true,
                "content": [{ "type": "text", "text": ERROR }] })
    };
    let called = |call_id: &str| json!({ "role": "assistant", "content": [{ "type": "function_call", "id": call_id }] });

    Scenario::new(
        ID,
        "repeated-failed-call-breaker",
        "The third identical call after two identical failures is refused without running the target.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-032")
            .allow_function(&fail),
    )
    .function(fail.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact([fail.tool()]),
            )
            .respond(Response::function_call("call-1", &fail, arguments.clone(), 8, 4)),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([Message::user(MESSAGE), called("call-1"), failed("call-1")])
                    .tools_exact([fail.tool()]),
            )
            .respond(Response::function_call("call-2", &fail, arguments.clone(), 8, 4)),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        Message::user(MESSAGE),
                        called("call-1"),
                        failed("call-1"),
                        called("call-2"),
                        failed("call-2"),
                    ])
                    .tools_exact([fail.tool()]),
            )
            .respond(Response::function_call("call-3", &fail, arguments.clone(), 8, 4)),
    )
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        Message::user(MESSAGE),
                        called("call-1"),
                        failed("call-1"),
                        called("call-2"),
                        failed("call-2"),
                        called("call-3"),
                        json!({ "role": "function_result", "function_call_id": "call-3",
                                "is_error": true,
                                "details": { "error": "repeated_failure", "count": 2 } }),
                    ])
                    .tools_exact([fail.tool()]),
            )
            .respond(Response::text("recorder is down", 18, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["recorder is down"])?;
        run.expect_function_calls("fail", 2)?;
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::script::JsonMatcherV1;

    #[test]
    fn the_last_generation_expects_the_local_repeated_failure_result() {
        let fixture = scenario();
        let JsonMatcherV1::Subset { expected, .. } = &fixture.script.generations[3].match_.messages
        else {
            panic!("the breaker generation must pin a subset of the history")
        };
        let refused = &expected.as_array().unwrap()[6];
        assert_eq!(refused["function_call_id"], "call-3");
        assert_eq!(refused["details"]["error"], "repeated_failure");
    }
}
