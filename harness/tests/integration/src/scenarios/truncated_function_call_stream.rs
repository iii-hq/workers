//! INT-024 — a provider stream cut inside a function call's arguments never
//! executes the call and never poisons the session.
//!
//! The scripted router streams text, opens a function call, delivers part of
//! its arguments, then ends the way every provider now ends a body cut
//! mid-arguments: one transient error frame whose partial carries the call
//! with degraded arguments. The Harness must keep the useful text, refuse to
//! dispatch the degraded call, resume once, and finish the turn with the
//! re-issued call executed exactly once. A follow-up message steered into the
//! same session during the final generation is then answered without any
//! restart of the provider, router, Harness or Console.

use serde_json::{json, Value};

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
    TruncatedCall,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const ID: &str = "INT-024";
const SLUG: &str = "truncated-function-call-stream";
const MESSAGE: &str = "Record the expected value once.";
const PARTIAL: &str = "Recording now.";
const CUT_CALL_ID: &str = "call-cut";
const CALL_ID: &str = "call-complete";
const FOLLOW_UP: &str = "Now confirm the session still works.";
const FOLLOW_UP_REPLY: &str = "still working";
const ERROR: &str = "fixture stream truncated: the body ended inside a function call's arguments [phase=sse-decode reason=open_function_call]";

pub(super) fn scenario() -> ScenarioFixture {
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
    .returns_text("recorded");
    let arguments = json!({ "value": "expected" });

    Scenario::new(
        ID,
        SLUG,
        "A stream cut inside function-call arguments keeps the partial text, never executes the degraded call, resumes once, runs the re-issued call exactly once, and the same session answers a follow-up without a restart.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-024")
            .allow_function(&record),
    )
    .function(record.clone())
    .terminal_turn_statuses(["completed"])
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::truncated_function_call(
                PARTIAL,
                TruncatedCall {
                    call_id: CUT_CALL_ID,
                    function: &record,
                    argument_delta: "{\"value\":\"exp",
                    degraded_arguments: json!({ "_raw": "{\"value\":\"exp" }),
                },
                ERROR,
                8,
                3,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([Message::user(MESSAGE)])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call(
                CALL_ID,
                &record,
                arguments.clone(),
                20,
                4,
            )),
    )
    .generation(
        // The follow-up parks while this generation streams and is delivered
        // by the completed path's steering check as the next step of the
        // same session.
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([Message::user(MESSAGE)])
                    .tools_exact([record.tool()]),
            )
            .parked_message(FOLLOW_UP)
            .respond(Response::text("recorded once", 30, 2)),
    )
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([Message::user(MESSAGE)])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text(FOLLOW_UP_REPLY, 40, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts([PARTIAL, "recorded once", FOLLOW_UP_REPLY])?;
        anyhow::ensure!(
            run.transcript.iter().any(|item| {
                item.get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                    .is_some_and(|blocks| {
                        blocks.iter().any(|b| b.get("text").and_then(Value::as_str) == Some(FOLLOW_UP))
                    })
            }),
            "the follow-up message must land in the same session: {:?}",
            run.transcript
        );
        run.expect_function_calls("record", 1)?;
        run.expect_call_payload("record", json!({ "value": "expected" }))?;
        run.expect_no_duplicate_messages()?;

        let cut_results = run
            .transcript
            .iter()
            .filter_map(|item| item.get("message"))
            .filter(|message| {
                message.get("role").and_then(Value::as_str) == Some("function_result")
                    && message.get("function_call_id").and_then(Value::as_str) == Some(CUT_CALL_ID)
            })
            .count();
        anyhow::ensure!(
            cut_results == 0,
            "the cut call must never produce a function result: {:?}",
            run.transcript
        );
        anyhow::ensure!(
            run.status.get("transient_resumes").and_then(Value::as_u64) == Some(1),
            "the cut stream must resume exactly once: {}",
            run.status
        );
        anyhow::ensure!(
            run.status.get("result_error").is_none_or(Value::is_null),
            "a recovered turn must not keep a terminal error: {}",
            run.status
        );
        let recoveries = run
            .transcript
            .iter()
            .filter_map(|item| item.get("custom"))
            .filter(|custom| custom.get("custom_type").and_then(Value::as_str) == Some("recovery"))
            .filter_map(|custom| custom.pointer("/data/status").and_then(Value::as_str))
            .collect::<Vec<_>>();
        anyhow::ensure!(
            recoveries == ["recovering", "recovered"],
            "recovery records must show one recovering/recovered pair: {recoveries:?}"
        );
        anyhow::ensure!(
            run.router_evidence
                .get("calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| calls.len() == 4),
            "recovery and the follow-up must consume exactly four generations: {}",
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
    use crate::types::frames::{AssistantMessageEvent, ContentBlock, ErrorKind, StopReason};

    #[test]
    fn fixture_ends_the_cut_stream_in_one_transient_error_with_a_degraded_call() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_turn_statuses, ["completed"]);

        let generation = &fixture.script.generations[0];
        assert_eq!(
            generation
                .frames
                .iter()
                .filter(|frame| frame.is_terminal())
                .count(),
            1
        );
        assert!(generation
            .frames
            .iter()
            .any(|frame| matches!(frame, AssistantMessageEvent::FunctioncallDelta { .. })));
        let Some(AssistantMessageEvent::Error { error }) = generation.frames.last() else {
            panic!("the cut stream must end in an error frame")
        };
        assert_eq!(error.stop_reason, StopReason::Error);
        assert_eq!(error.error_kind, Some(ErrorKind::Transient));
        assert_eq!(error.error_message.as_deref(), Some(ERROR));
        let degraded = error.content.iter().find_map(|block| match block {
            ContentBlock::FunctionCall { arguments, .. } => Some(arguments),
            _ => None,
        });
        assert!(degraded.is_some_and(|arguments| arguments.get("_raw").is_some()));
        assert!(!generation.response.ok);
        assert_eq!(
            generation
                .response
                .error
                .as_ref()
                .map(|error| error.code.as_str()),
            Some("transient")
        );
    }
}
