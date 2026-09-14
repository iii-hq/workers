//! MOT-4776: keepalives followed by a startup timeout must expose a durable
//! failure immediately, with no fabricated partial or automatic resume.

use serde_json::Value;

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;
use crate::types::frames::ErrorKind;

const MESSAGE: &str = "Exercise a provider that never starts generating.";
const ERROR: &str = "DeepSeek model deepseek-flash did not start generating within 120000ms. The request may be queued upstream. Try again or choose another model.";
const SUMMARY: &str = "The provider temporarily failed while generating a response.";

pub(super) fn scenario() -> ScenarioFixture {
    Scenario::new(
        "INT-029",
        "provider-startup-timeout",
        "A provider startup timeout persists one visible failure without resuming an empty response.",
        ScenarioDriver::Direct,
        Model::scripted("deepseek-flash"),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-029")
            .without_functions(),
    )
    .terminal_turn_statuses(["failed"])
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .without_tools(),
            )
            .respond(Response::startup_error(ERROR, ErrorKind::Transient)),
    )
    .verify(|run| {
        run.expect_assistant_texts(std::iter::empty::<&str>())?;
        run.expect_message_counts(1, 1, 0)?;
        run.expect_no_duplicate_messages()?;
        anyhow::ensure!(
            run.status.get("result_error").and_then(Value::as_str) == Some(SUMMARY),
            "startup failure did not expose a stable public summary: {}", run.status
        );
        anyhow::ensure!(
            run.status.get("partial_result_available").and_then(Value::as_bool) == Some(false)
                && run.status.get("transient_resumes").and_then(Value::as_u64) == Some(0),
            "startup failure must not fabricate a partial or resume: {}", run.status
        );
        let errors = run.transcript.iter().filter_map(|item| {
            let custom = item.get("custom")?;
            (custom.get("custom_type").and_then(Value::as_str) == Some("error"))
                .then_some(&custom["data"])
        }).collect::<Vec<_>>();
        anyhow::ensure!(errors.len() == 1, "expected one durable failure: {:?}", run.transcript);
        let data = errors[0];
        anyhow::ensure!(
            data.get("summary").and_then(Value::as_str) == Some(SUMMARY)
                && data.get("detail").and_then(Value::as_str).is_some_and(|detail| detail.contains(ERROR))
                && data.get("class").and_then(Value::as_str) == Some("llm.transient")
                && data.get("retryable").and_then(Value::as_bool) == Some(true)
                && data.get("partial_result_available").and_then(Value::as_bool) == Some(false),
            "durable failure lost its diagnostics or manual retry option: {data}"
        );
        anyhow::ensure!(
            !run.transcript.iter().any(|item| item.pointer("/custom/custom_type").and_then(Value::as_str) == Some("recovery")),
            "there is no partial response to resume"
        );
        anyhow::ensure!(
            run.router_evidence.get("calls").and_then(Value::as_array).is_some_and(|calls| calls.len() == 1),
            "startup failure was retried: {}", run.router_evidence
        );
        Ok(())
    })
    .scenario_timeout_ms(60_000)
    .build()
}
