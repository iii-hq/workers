//! A retryable startup failure after tool execution resumes the same turn.

use serde_json::{Value, json};

use super::ScenarioDriver;
use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use crate::fixtures::ScenarioFixture;
use crate::types::frames::ErrorKind;

const MESSAGE: &str = "Record the value once and confirm it.";
const ERROR: &str = "DeepSeek model deepseek-flash did not start generating within 120000ms.";
const CALL_ID: &str = "call-before-timeout";

pub(super) fn scenario() -> ScenarioFixture {
    let model = Model::scripted("deepseek-flash");
    let record = ControlledFunction::new("{{run_id}}::record", "Record the value.")
        .request_schema(json!({ "type": "object", "properties": {} }))
        .returns_text("recorded");
    let request = || {
        Request::new()
            .turn_request()
            .system_prompt_sha256("{{system_prompt_sha256}}")
            .tools_exact([record.tool()])
    };
    let after_call = || {
        request().messages_exact([
            Message::user(MESSAGE),
            Message::function_call(CALL_ID, &record, json!({}), &model, 8, 4),
            Message::function_result(CALL_ID, &record, "recorded"),
        ])
    };

    Scenario::new(
        "INT-029",
        "provider-startup-timeout",
        "The turn retries a transient startup failure with the completed function result.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-029")
            .allow_function(&record),
    )
    .function(record.clone())
    .generation(
        Generation::new(1)
            .expect(request().messages_exact([Message::user(MESSAGE)]))
            .respond(Response::function_call(CALL_ID, &record, json!({}), 8, 4)),
    )
    .generation(
        Generation::new(2)
            .expect(after_call())
            .respond(Response::startup_error(ERROR, ErrorKind::Transient)),
    )
    .generation(
        Generation::new(3)
            .expect(after_call())
            .respond(Response::text("Recorded once.", 18, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["Recorded once."])?;
        run.expect_function_calls("record", 1)?;
        run.expect_message_counts(1, 3, 1)?;
        run.expect_no_duplicate_messages()?;
        anyhow::ensure!(run.status["transient_resumes"] == 1, "{}", run.status);
        let recovery = run
            .transcript
            .iter()
            .filter_map(|item| {
                (item.pointer("/custom/custom_type")?.as_str()? == "recovery")
                    .then_some(&item["custom"]["data"])
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(
            recovery
                .iter()
                .filter_map(|data| data["status"].as_str())
                .collect::<Vec<_>>()
                == ["recovering", "recovered"],
            "unexpected recovery events: {recovery:?}"
        );
        anyhow::ensure!(
            recovery[0]["partial_result_available"] == false
                && recovery[0]["detail"]
                    .as_str()
                    .is_some_and(|detail| detail.contains(ERROR)),
            "unexpected startup diagnostics: {}",
            recovery[0]
        );
        anyhow::ensure!(
            !run.transcript.iter().any(|item| item
                .pointer("/custom/custom_type")
                .and_then(Value::as_str)
                == Some("error")),
            "a recovered turn recorded a terminal error"
        );
        Ok(())
    })
    .scenario_timeout_ms(60_000)
    .build()
}
