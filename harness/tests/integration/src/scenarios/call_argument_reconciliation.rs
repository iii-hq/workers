//! INT-033 — a call whose arguments carry stringified JSON the target's
//! schema rejects (`"true"` for a boolean, `"5"` for an integer) is repaired
//! before dispatch: the target receives the typed values, the call succeeds
//! on the first attempt, and the result says the values were parsed and the
//! call ran as intended. The same repair again in the turn runs the same way
//! and keeps its `reconciled` origin annotation, but its result carries no
//! second notice.
//!
//! Deterministic regression coverage for MOT-4847 (layer A).

use serde_json::{json, Value};

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::evidence_data::message_text;
use crate::fixtures::ScenarioFixture;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-033";
    const MESSAGE: &str = "Look up retry twice.";
    const RESULT: &str = "found 1 entry";

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
    .returns_text(RESULT);
    let stringified = json!({ "query": "retry", "exact": "true", "limit": "5" });
    let called = |call_id: &str| {
        json!({ "role": "assistant", "content": [{
            "type": "function_call",
            "id": call_id,
            "function_id": "{{run_id}}::lookup"
        }]})
    };
    let succeeded = |call_id: &str| {
        json!({
            "role": "function_result",
            "function_call_id": call_id,
            "function_id": "{{run_id}}::lookup",
            "is_error": false
        })
    };

    Scenario::new(
        ID,
        "call-argument-reconciliation",
        "Stringified JSON the target's schema rejects is parsed before dispatch; the result notes the parse once per turn.",
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
                "call-1",
                &lookup,
                stringified.clone(),
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
                        called("call-1"),
                        succeeded("call-1"),
                    ])
                    .tools_exact([lookup.tool()]),
            )
            .respond(Response::function_call(
                "call-2",
                &lookup,
                stringified.clone(),
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
                        json!({ "role": "user" }),
                        called("call-1"),
                        succeeded("call-1"),
                        called("call-2"),
                        succeeded("call-2"),
                    ])
                    .tools_exact([lookup.tool()]),
            )
            .respond(Response::text("done", 20, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["done"])?;
        run.expect_function_calls("lookup", 2)?;
        // Both calls ran once each, with the typed values.
        let typed = json!({ "query": "retry", "exact": true, "limit": 5 });
        for call in run.calls("lookup") {
            anyhow::ensure!(
                call.payload.as_ref() == Some(&typed),
                "lookup payload {:?} != {typed}",
                call.payload
            );
        }
        let function_id = format!("{}::lookup", run.run_id);
        let results = run.function_results(&function_id);
        anyhow::ensure!(
            results.len() == 2,
            "reconciled calls have {} function results, expected 2",
            results.len()
        );
        let first = message_text(results[0]);
        anyhow::ensure!(
            first.contains(RESULT)
                && first.contains("`exact` (boolean)")
                && first.contains("`limit` (integer)")
                && first.contains("parsed before dispatch; the call ran as intended"),
            "first result does not carry the target output and the parse note: {first}"
        );
        anyhow::ensure!(
            !first.contains("Send arguments") && !first.contains('→'),
            "a lossless parse note carries an instruction or value previews: {first}"
        );
        let second = message_text(results[1]);
        anyhow::ensure!(
            second == RESULT,
            "the repeated repair was noted again: {second}"
        );
        // Both results still record what was parsed on the entry origin.
        let annotated = run
            .transcript
            .iter()
            .filter(|item| {
                item.pointer("/message/role").and_then(Value::as_str) == Some("function_result")
                    && item.pointer("/message/function_id").and_then(Value::as_str)
                        == Some(function_id.as_str())
            })
            .filter(|item| {
                item.pointer("/origin/reconciled")
                    .and_then(Value::as_array)
                    .is_some_and(|changes| changes.len() == 2)
            })
            .count();
        anyhow::ensure!(
            annotated == 2,
            "{annotated} of 2 results carry the `reconciled` origin annotation"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}
