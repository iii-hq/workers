//! INT-040 — through the `agent_trigger` wrapper the model holds no request
//! schema until it fetches the contract, so a malformed call's diagnosis
//! carries the schema itself and the retry needs no `engine::functions::info`
//! step. Once the contract is in context, a later diagnosis only points at it.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::evidence_data::message_text;
use crate::fixtures::ScenarioFixture;

const AGENT_TRIGGER: &str = "agent_trigger";
const INFO: &str = "engine::functions::info";
const LOOKUP: &str = "{{run_id}}::lookup";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-040";
    const MESSAGE: &str = "Look up retry, many results.";
    const ERROR: &str = "serialization error: invalid type: string \"many\", expected an integer";
    const DIAGNOSIS: &str = "[harness] These arguments do not match";
    const ATTACHED: &str = "no need to fetch the contract";
    const POINTER: &str = "engine::functions::info shows the full contract";

    let model = Model::scripted("fixture-model");
    let lookup =
        ControlledFunction::new(LOOKUP, "Look up integration fixture entries; always fails.")
            .request_schema(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["query"]
            }))
            .returns_error(ERROR);
    let wrapped = |call_id: &str, function: &str, payload: serde_json::Value| {
        Response::function_calls_raw(
            vec![(
                call_id,
                AGENT_TRIGGER,
                json!({ "function": function, "payload": payload }),
            )],
            8,
            4,
        )
    };
    let called = |call_id: &str| json!({ "role": "assistant", "content": [{ "type": "function_call", "id": call_id }] });
    let answered =
        |call_id: &str| json!({ "role": "function_result", "function_call_id": call_id });

    Scenario::new(
        ID,
        "call-argument-contract",
        "A malformed agent_trigger call whose contract the model never fetched gets the request schema in its diagnosis; after a contract fetch, the next diagnosis only points at it.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-040")
            .allow_function(&lookup)
            .allow_id(INFO)
            .agent_trigger(),
    )
    .function(lookup.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_subset([]),
            )
            .respond(wrapped(
                "call-1",
                LOOKUP,
                json!({ "query": "retry", "limit": "many" }),
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([Message::user(MESSAGE), called("call-1"), answered("call-1")])
                    .tools_subset([]),
            )
            .respond(wrapped(
                "call-info",
                INFO,
                json!({ "function_id": LOOKUP }),
            )),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(2)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        Message::user(MESSAGE),
                        called("call-1"),
                        answered("call-1"),
                        called("call-info"),
                        answered("call-info"),
                    ])
                    .tools_subset([]),
            )
            .respond(wrapped(
                "call-2",
                LOOKUP,
                json!({ "query": "again", "limit": "lots" }),
            )),
    )
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(3)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        Message::user(MESSAGE),
                        called("call-1"),
                        answered("call-1"),
                        called("call-info"),
                        answered("call-info"),
                        called("call-2"),
                        answered("call-2"),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("lookup is down", 18, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["lookup is down"])?;
        run.expect_function_calls("lookup", 2)?;
        let function_id = format!("{}::lookup", run.run_id);
        let results = run.function_results(&function_id);
        anyhow::ensure!(
            results.len() == 2,
            "lookup has {} function results, expected 2",
            results.len()
        );
        let first = message_text(results[0]);
        anyhow::ensure!(
            first.contains(ERROR) && first.contains(DIAGNOSIS) && first.contains("`limit`"),
            "first result does not carry the target error and the diagnosis: {first}"
        );
        anyhow::ensure!(
            first.contains(ATTACHED)
                && first.contains(r#""limit":{"type":"integer"}"#)
                && !first.contains(POINTER),
            "a contract never fetched must ride along in the diagnosis: {first}"
        );
        let second = message_text(results[1]);
        anyhow::ensure!(
            second.contains(DIAGNOSIS) && second.contains(POINTER) && !second.contains(ATTACHED),
            "a fetched contract must not be sent again with the diagnosis: {second}"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}
