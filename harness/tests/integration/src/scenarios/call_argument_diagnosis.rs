//! INT-034 — a call whose arguments the harness cannot repair (`"many"` for
//! an integer) still runs; when the target rejects them as malformed, the
//! result names the violating field, so the model fixes it in one retry. A
//! retry with valid arguments that fails the same way gets no diagnosis.
//!
//! Deterministic regression coverage for MOT-4847 (layer C).

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::evidence_data::message_text;
use crate::fixtures::ScenarioFixture;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-034";
    const MESSAGE: &str = "Look up retry, many results.";
    const ERROR: &str = "serialization error: invalid type: string \"many\", expected an integer";
    const DIAGNOSIS: &str = "[harness] These arguments do not match";

    let model = Model::scripted("fixture-model");
    let lookup = ControlledFunction::new(
        "{{run_id}}::lookup",
        "Look up integration fixture entries; always fails.",
    )
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
    let called = |call_id: &str| json!({ "role": "assistant", "content": [{ "type": "function_call", "id": call_id }] });
    let failed = |call_id: &str| {
        json!({ "role": "function_result", "function_call_id": call_id,
                "function_id": "{{run_id}}::lookup", "is_error": true })
    };

    Scenario::new(
        ID,
        "call-argument-diagnosis",
        "A malformed call the harness cannot repair fails with the violating field named; a valid retry gets no diagnosis.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-034")
            .allow_function(&lookup),
    )
    .function(lookup.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact([lookup.tool()]),
            )
            .respond(Response::function_call(
                "call-1",
                &lookup,
                json!({ "query": "retry", "limit": "many" }),
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
                    .messages_subset([Message::user(MESSAGE), called("call-1"), failed("call-1")])
                    .tools_exact([lookup.tool()]),
            )
            .respond(Response::function_call(
                "call-2",
                &lookup,
                json!({ "query": "retry", "limit": 5 }),
                8,
                4,
            )),
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
                    .tools_exact([lookup.tool()]),
            )
            .respond(Response::text("lookup is down", 18, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["lookup is down"])?;
        run.expect_function_calls("lookup", 2)?;
        // The unrepairable arguments reached the target untouched.
        run.expect_call_payload("lookup", json!({ "query": "retry", "limit": "many" }))?;
        let function_id = format!("{}::lookup", run.run_id);
        let results = run.function_results(&function_id);
        anyhow::ensure!(
            results.len() == 2,
            "lookup has {} function results, expected 2",
            results.len()
        );
        let first = message_text(results[0]);
        anyhow::ensure!(
            first.contains(ERROR) && first.contains(DIAGNOSIS),
            "first result does not carry the target error and the diagnosis: {first}"
        );
        anyhow::ensure!(
            first.contains("`limit`") && first.contains("integer"),
            "diagnosis does not name the violating field: {first}"
        );
        let second = message_text(results[1]);
        anyhow::ensure!(
            second.contains(ERROR) && !second.contains(DIAGNOSIS),
            "valid arguments must fail without a diagnosis: {second}"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}
