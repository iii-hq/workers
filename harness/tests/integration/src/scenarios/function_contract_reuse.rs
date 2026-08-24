//! INT-025 — unchanged function contracts reuse a retained full result.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const INFO: &str = "engine::functions::info";
const FULL_CONTRACT: &str = r#"{"description":"Record one integration fixture value.","function_id":"{{run_id}}::record","registered_triggers":[],"request_schema":{"properties":{"value":{"type":"string"}},"required":["value"],"type":"object"},"response_schema":{"$schema":"http://json-schema.org/draft-07/schema#","const":{"content":[{"text":"recorded","type":"text"}],"is_error":false}},"worker_name":"integration-probe"}"#;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-025";
    const MESSAGE: &str = "Fetch the recorder contract twice.";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new(
        "{{run_id}}::record",
        "Record one integration fixture value.",
    )
    .request_schema(json!({
        "type": "object",
        "properties": { "value": { "type": "string" } },
        "required": ["value"]
    }))
    .returns_text("recorded");
    let info_arguments = json!({ "function_id": "{{run_id}}::record" });

    Scenario::new(
        ID,
        "function-contract-reuse",
        "A repeated contract lookup still invokes the engine and retains details, but sends the model an unchanged marker while the exact first result remains in context.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .allow_function(&record)
            .allow_id(INFO),
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
            .respond(Response::function_call_raw(
                "call-info-1",
                INFO,
                info_arguments.clone(),
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
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [{
                            "type": "function_call", "id": "call-info-1", "function_id": INFO
                        }] }),
                        json!({
                            "role": "function_result",
                            "function_call_id": "call-info-1",
                            "function_id": INFO,
                            "is_error": false,
                            "content": [{ "type": "text", "text": FULL_CONTRACT }],
                            "details": { "function_id": "{{run_id}}::record" }
                        }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-info-2",
                INFO,
                info_arguments,
                8,
                4,
            )),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(2)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({
                            "role": "function_result",
                            "function_call_id": "call-info-1",
                            "details": { "function_id": "{{run_id}}::record" }
                        }),
                        json!({ "role": "assistant", "content": [{
                            "type": "function_call", "id": "call-info-2", "function_id": INFO
                        }] }),
                        json!({
                            "role": "function_result",
                            "function_call_id": "call-info-2",
                            "function_id": INFO,
                            "is_error": false,
                            "content": [{
                                "type": "text",
                                "text": "{\"contract_status\":\"unchanged_in_context\",\"function_id\":\"{{run_id}}::record\",\"source_function_call_id\":\"call-info-1\"}"
                            }],
                            "details": { "function_id": "{{run_id}}::record" }
                        }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("contract reused", 8, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["contract reused"])?;
        let info_calls = run.spans_named("call engine::functions::info");
        anyhow::ensure!(
            info_calls.len() == 2,
            "engine::functions::info ran {} times, expected 2",
            info_calls.len()
        );
        run.expect_target_calls(0)?;
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_requires_full_then_marker_provider_visible_results() {
        let fixture = scenario();
        fixture.validate().unwrap();
        let first_gate =
            serde_json::to_string(&fixture.script.generations[1].match_.messages).unwrap();
        assert!(first_gate.contains("\"content\""), "{first_gate}");
        assert!(first_gate.contains("request_schema"), "{first_gate}");
        assert!(first_gate.contains("response_schema"), "{first_gate}");

        let gate = serde_json::to_string(&fixture.script.generations[2].match_.messages).unwrap();
        assert!(gate.contains("call-info-2"), "{gate}");
        assert!(gate.contains("unchanged_in_context"), "{gate}");
        assert!(gate.contains("call-info-1"), "{gate}");
    }
}
