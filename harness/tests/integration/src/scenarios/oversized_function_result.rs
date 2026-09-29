//! INT-030 — a function result over `max_result_bytes` is elided at capture
//! time and the turn completes, instead of the oversized echo tripping the
//! engine's per-message transport cap into a permanent reconnect loop.
//!
//! Deterministic regression coverage for MOT-4498.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::evidence_data::message_text;
use crate::fixtures::ScenarioFixture;

/// Past the 256 KiB default cap once `content` and `details` both carry it.
const PAYLOAD_CHARS: usize = 150_000;

/// INT-030: the controlled function returns more than the default cap; the
/// model must see the elision marker and still finish the turn.
pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-030";
    const MESSAGE: &str = "Dump the recorder once.";
    const CALL_ID: &str = "call-1";

    let model = Model::scripted("fixture-model");
    let payload = "x".repeat(PAYLOAD_CHARS);
    let record = ControlledFunction::new(
        "{{run_id}}::record",
        "Return one oversized integration fixture value.",
    )
    .request_schema(json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "value": { "type": "string" } },
        "required": ["value"]
    }))
    .returns_text(&payload);
    let arguments = json!({ "value": "expected" });

    Scenario::new(
        ID,
        "oversized-function-result",
        "A function result over the capture-time byte cap reaches the model as an elision marker and the turn completes.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-030")
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
    // The provider request must carry the marker, never the raw payload: the
    // cap is applied before the result is written, so the session entry and
    // the assembled context agree.
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
                            "function_id": "{{run_id}}::record"
                        }]}),
                        json!({
                            "role": "function_result",
                            "function_call_id": CALL_ID,
                            "function_id": "{{run_id}}::record",
                            "is_error": false,
                            "details": { "result_capped": { "max_bytes": 262_144 } }
                        }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("summarised", 20, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["summarised"])?;
        run.expect_function_calls("record", 1)?;
        run.expect_call_payload("record", json!({ "value": "expected" }))?;
        let function_id = format!("{}::record", run.run_id);
        let results = run.function_results(&function_id);
        anyhow::ensure!(
            results.len() == 1,
            "oversized call has {} closing function results, expected 1",
            results.len()
        );
        let text = message_text(results[0]);
        anyhow::ensure!(
            text.starts_with("<omitted: result was ~"),
            "function result was not elided: {}",
            &text[..text.len().min(200)]
        );
        anyhow::ensure!(
            text.len() < 1_024,
            "elided function result is still {} bytes",
            text.len()
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
    fn fixture_response_exceeds_the_default_cap_and_pins_the_marker() {
        let fixture = scenario();
        let target = fixture.scenario.target.as_ref().unwrap();
        let text = target.response["content"][0]["text"].as_str().unwrap();
        // content + details each carry the payload, so 2x must clear 256 KiB.
        assert!(text.len() * 2 > 262_144);
        assert!(matches!(
            fixture.script.generations[1].match_.messages,
            JsonMatcherV1::Subset { .. }
        ));
    }
}
