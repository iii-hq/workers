//! INT-021 — partial provider output ends in one authoritative router error.
//!
//! The scripted router sends content in multiple deltas with keepalive noise,
//! reports a non-terminal stop, then closes with one permanent error frame and
//! a failed RPC response. The Harness must preserve the useful partial, refuse
//! to resume a permanent failure, and reach one durable terminal failure with
//! no queued work or pending spans.

use serde_json::Value;

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;
use crate::types::frames::ErrorKind;

const ID: &str = "INT-021";
const SLUG: &str = "router-midstream-terminal-error";
const MESSAGE: &str = "Return an answer that exercises terminal failure handling.";
const PARTIAL: &str = "useful partial answer";
const ERROR: &str = "provider disappeared after streaming content";
const PUBLIC_SUMMARY: &str = "The provider rejected this request.";

pub(super) fn scenario() -> ScenarioFixture {
    Scenario::new(
        ID,
        SLUG,
        "A permanent router error after partial output terminates the turn without losing the partial or leaving work pending.",
        ScenarioDriver::Direct,
        Model::scripted("fixture-model"),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-021")
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
            .respond(Response::terminal_error_after_text(
                PARTIAL,
                ["useful ", "partial ", "answer"],
                ERROR,
                ErrorKind::Permanent,
                12,
                3,
            )),
    )
    .verify(|run| {
        run.expect_assistant_texts([PARTIAL])?;
        run.expect_message_counts(1, 1, 0)?;
        run.expect_no_duplicate_messages()?;

        anyhow::ensure!(
            run.status.get("result_error").and_then(Value::as_str) == Some(PUBLIC_SUMMARY),
            "terminal error status did not use stable public copy: {}",
            run.status
        );
        let durable_errors = run
            .transcript
            .iter()
            .filter_map(|item| {
                let custom = item.get("custom")?;
                (custom.get("custom_type").and_then(Value::as_str) == Some("error"))
                    .then_some(custom)
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(
            durable_errors.len() == 1,
            "expected one durable error record, found {}: {:?}",
            durable_errors.len(),
            run.transcript
        );
        let error_data = durable_errors[0]
            .get("data")
            .cloned()
            .unwrap_or(Value::Null);
        anyhow::ensure!(
            error_data.get("summary").and_then(Value::as_str) == Some(PUBLIC_SUMMARY),
            "durable error summary did not use stable public copy: {error_data}"
        );
        anyhow::ensure!(
            error_data
                .get("detail")
                .and_then(Value::as_str)
                .is_some_and(|detail| detail.contains(ERROR)),
            "durable error detail did not preserve the router diagnostic: {error_data}"
        );
        anyhow::ensure!(
            error_data
                .get("reason")
                .and_then(Value::as_str)
                .is_some_and(|reason| reason.contains(ERROR)),
            "compatibility reason did not preserve the router diagnostic: {error_data}"
        );
        anyhow::ensure!(
            error_data.get("class").and_then(Value::as_str) == Some("llm.permanent")
                && error_data.get("retryable").and_then(Value::as_bool) == Some(false),
            "terminal error was not durably classified as permanent: {error_data}"
        );
        anyhow::ensure!(
            run.status
                .get("partial_result_available")
                .and_then(Value::as_bool)
                == Some(true),
            "failed turn did not retain its partial result: {}",
            run.status
        );
        anyhow::ensure!(
            run.status.get("transient_resumes").and_then(Value::as_u64) == Some(0),
            "permanent terminal failure must not resume: {}",
            run.status
        );
        anyhow::ensure!(
            run.router_evidence
                .pointer("/calls/0/outcome")
                .and_then(Value::as_str)
                == Some("matched")
                && run
                    .router_evidence
                    .get("calls")
                    .and_then(Value::as_array)
                    .is_some_and(|calls| calls.len() == 1),
            "adversarial generation was not served exactly once: {}",
            run.router_evidence
        );
        Ok(())
    })
    .scenario_timeout_ms(60_000)
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::frames::{AssistantMessageEvent, ContentBlock, StopReason};

    #[test]
    fn fixture_streams_partial_keepalives_then_one_permanent_terminal() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.expected_turn_statuses, ["failed"]);

        let generation = &fixture.script.generations[0];
        assert_eq!(
            generation
                .frames
                .iter()
                .filter(|frame| frame.is_terminal())
                .count(),
            1
        );
        assert!(
            generation
                .frames
                .iter()
                .filter(|frame| matches!(frame, AssistantMessageEvent::Ping))
                .count()
                >= 2
        );
        let Some(AssistantMessageEvent::Error { error }) = generation.frames.last() else {
            panic!("adversarial stream must end in an error frame")
        };
        assert_eq!(error.stop_reason, StopReason::Error);
        assert_eq!(error.error_kind, Some(ErrorKind::Permanent));
        assert_eq!(error.error_message.as_deref(), Some(ERROR));
        assert_eq!(
            error.content,
            vec![ContentBlock::Text {
                text: PARTIAL.to_string()
            }]
        );
        assert!(!generation.response.ok);
        assert_eq!(generation.response.stop_reason, Some(StopReason::Error));
        assert_eq!(
            generation
                .response
                .error
                .as_ref()
                .map(|error| error.code.as_str()),
            Some("permanent")
        );
    }
}
