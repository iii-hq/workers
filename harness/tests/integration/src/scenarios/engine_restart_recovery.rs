//! INT-010 — an engine restart cannot strand an interrupted function call.
//!
//! Deterministic regression coverage for
//! <https://github.com/iii-hq/workers/issues/507>.

use serde_json::json;

use super::dsl::{ControlledFunction, Generation, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::evidence_data::message_text;
use crate::fixtures::ScenarioFixture;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-010";
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
    .returns_text("recorded")
    .hold_response();
    let arguments = json!({ "value": "expected" });

    Scenario::new(
        ID,
        "crash-recovery-507",
        "A conversation stays usable when the engine restarts during a function call.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-010")
            .allow_function(&record),
    )
    .function(record.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([super::dsl::Message::user(MESSAGE)])
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
    // Recovery must continue from the authoritative durable turn record: the
    // original prompt, tool policy, and exact turn step survive the engine.
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(1)
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
                            "is_error": true,
                            "details": { "error": { "code": "engine_restart" } }
                        }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("recovered", 20, 2)),
    )
    .engine_sigkill(&record, 1, 1_500)
    .scenario_timeout_ms(120_000)
    .verify(|run| {
        run.expect_assistant_texts(["recovered"])?;
        run.expect_message_counts(1, 2, 1)?;
        run.expect_target_calls(1)?;
        anyhow::ensure!(
            run.target_calls == [json!({ "value": "expected" })],
            "controlled function payloads {:?} != expected",
            run.target_calls
        );

        let function_id = format!("{}::record", run.run_id);
        let results = run.function_results(&function_id);
        anyhow::ensure!(
            results.len() == 1,
            "interrupted call has {} closing function results, expected 1",
            results.len()
        );
        let result = results[0];
        anyhow::ensure!(
            result
                .get("function_call_id")
                .and_then(serde_json::Value::as_str)
                == Some(CALL_ID),
            "function result does not close {CALL_ID}: {result}"
        );
        anyhow::ensure!(
            result.get("is_error").and_then(serde_json::Value::as_bool) == Some(true),
            "interrupted function result must be an error: {result}"
        );
        anyhow::ensure!(
            result
                .pointer("/details/error/code")
                .and_then(serde_json::Value::as_str)
                == Some("engine_restart"),
            "interrupted function result must identify engine_restart: {result}"
        );
        anyhow::ensure!(
            message_text(result).contains("execution interrupted by engine restart"),
            "interrupted function result lacks the recovery explanation: {result}"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::scenario::FaultKind;
    use crate::types::script::JsonMatcherV1;

    #[test]
    fn fixture_holds_the_first_call_and_requires_a_real_engine_restart() {
        let fixture = scenario();
        let target = fixture.scenario.target.as_ref().unwrap();
        let fault = fixture.scenario.fault.as_ref().unwrap();

        assert!(target.hold_response);
        assert_eq!(fault.kind, FaultKind::EngineSigkill);
        assert_eq!(fault.function_id, target.function_id);
        assert_eq!(fault.after_target_calls, 1);
        assert_eq!(fault.restart_delay_ms, 1_500);
        assert_eq!(fixture.scenario.deadlines.scenario_ms, 120_000);
        assert!(matches!(
            fixture.script.generations[1].match_.request_id,
            JsonMatcherV1::Regex { .. }
        ));
    }
}
