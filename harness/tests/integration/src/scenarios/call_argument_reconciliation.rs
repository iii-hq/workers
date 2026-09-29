//! INT-033 — a call whose arguments carry stringified JSON the target's
//! schema rejects (`"true"` for a boolean, `"5"` for an integer) is repaired
//! before dispatch: the target receives the typed values, the call succeeds
//! on the first attempt, and the result tells the model what was reconciled.
//!
//! Deterministic regression coverage for MOT-4847 (layer A).

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::evidence_data::message_text;
use crate::fixtures::ScenarioFixture;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-033";
    const MESSAGE: &str = "Look up retry once.";
    const CALL_ID: &str = "call-1";

    let model = Model::scripted("fixture-model");
    let lookup = ControlledFunction::new(
        "{{run_id}}::lookup",
        "Look up one integration fixture entry.",
    )
    .request_schema(json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": { "type": "string" },
            "exact": { "type": "boolean" },
            "limit": { "type": "integer" }
        },
        "required": ["query"]
    }))
    .returns_text("found 1 entry");

    Scenario::new(
        ID,
        "call-argument-reconciliation",
        "Stringified JSON the target's schema rejects is parsed before dispatch and the result notes the repair.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-033")
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
                CALL_ID,
                &lookup,
                json!({ "query": "retry", "exact": "true", "limit": "5" }),
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
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [{
                            "type": "function_call",
                            "id": CALL_ID,
                            "function_id": "{{run_id}}::lookup"
                        }]}),
                        json!({
                            "role": "function_result",
                            "function_call_id": CALL_ID,
                            "function_id": "{{run_id}}::lookup",
                            "is_error": false
                        }),
                    ])
                    .tools_exact([lookup.tool()]),
            )
            .respond(Response::text("done", 20, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["done"])?;
        run.expect_function_calls("lookup", 1)?;
        // The target ran once, with the typed values.
        run.expect_call_payload(
            "lookup",
            json!({ "query": "retry", "exact": true, "limit": 5 }),
        )?;
        let function_id = format!("{}::lookup", run.run_id);
        let results = run.function_results(&function_id);
        anyhow::ensure!(
            results.len() == 1,
            "reconciled call has {} function results, expected 1",
            results.len()
        );
        let text = message_text(results[0]);
        anyhow::ensure!(
            text.contains("found 1 entry") && text.contains("arguments were reconciled"),
            "result does not carry the target output and the reconciliation note: {text}"
        );
        anyhow::ensure!(
            text.contains("`exact` \"true\" → true") && text.contains("`limit` \"5\" → 5"),
            "note does not name both repairs: {text}"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}
