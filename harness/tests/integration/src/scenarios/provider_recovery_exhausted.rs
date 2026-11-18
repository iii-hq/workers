//! A partial answer survives repeated reconnect failures until the retry budget runs out.

use serde_json::json;

use super::ScenarioDriver;
use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use crate::fixtures::ScenarioFixture;
use crate::types::frames::ErrorKind;

const MESSAGE: &str = "Describe the result.";
const PARTIAL: &str = "The first observation";
const RECONNECT_ERROR: &str = "connection reset before the first token";

pub(super) fn scenario() -> ScenarioFixture {
    let request = || {
        Request::new()
            .turn_request()
            .system_prompt_sha256("{{system_prompt_sha256}}")
            .without_tools()
    };
    let mut scenario = Scenario::new(
        "INT-044",
        "provider-recovery-exhausted",
        "The turn keeps its partial answer when all three reconnect attempts fail.",
        ScenarioDriver::Direct,
        Model::scripted("fixture-model"),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-044")
            .without_functions(),
    )
    .terminal_turn_statuses(["failed"])
    .generation(
        Generation::new(1)
            .expect(request().messages_exact([Message::user(MESSAGE)]))
            .respond(Response::terminal_error_after_text(
                PARTIAL,
                [PARTIAL],
                "stream interrupted",
                ErrorKind::Transient,
                8,
                2,
            )),
    );

    for ordinal in 2..=4 {
        scenario = scenario.generation(
            Generation::new(ordinal)
                .expect(request().messages_subset([
                    Message::user(MESSAGE),
                    json!({"role": "assistant", "content": [{"type": "text", "text": PARTIAL}]}),
                    json!({"role": "user"}),
                ]))
                .respond(Response::startup_error(
                    RECONNECT_ERROR,
                    ErrorKind::Transient,
                )),
        );
    }
    scenario
        .verify(|run| {
            run.expect_assistant_texts([PARTIAL])?;
            run.expect_message_counts(2, 4, 0)?;
            run.expect_no_duplicate_messages()?;
            anyhow::ensure!(
                run.status["transient_resumes"] == 3 && run.status["result"] == PARTIAL,
                "{}",
                run.status
            );
            let custom = |kind: &str| {
                run.transcript
                    .iter()
                    .filter(|item| item["custom"]["custom_type"] == kind)
                    .map(|item| &item["custom"]["data"])
                    .collect::<Vec<_>>()
            };
            let attempts = custom("recovery");
            anyhow::ensure!(
                attempts.len() == 3,
                "unexpected recovery events: {attempts:?}"
            );
            for (index, data) in attempts.iter().enumerate() {
                anyhow::ensure!(
                    data["status"] == "recovering"
                        && data["attempt"] == (index + 1)
                        && data["partial_result_available"] == true,
                    "unexpected recovery: {data}"
                );
            }
            let errors = custom("error");
            let [error] = errors.as_slice() else {
                anyhow::bail!("expected one terminal error: {errors:?}")
            };
            anyhow::ensure!(
                error["retryable"] == true
                    && error["partial_result_available"] == true
                    && error["detail"]
                        .as_str()
                        .is_some_and(|detail| detail.contains(RECONNECT_ERROR))
                    && error["recovery"]
                        == json!({"attempted": 3, "max_attempts": 3, "outcome": "exhausted"}),
                "unexpected terminal error: {error}"
            );
            Ok(())
        })
        .scenario_timeout_ms(60_000)
        .build()
}
