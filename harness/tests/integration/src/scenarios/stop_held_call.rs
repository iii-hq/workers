//! INT-039 — `harness::stop` ends a turn whose function call never returns.
//!
//! The controlled function holds its response. The runner waits until it is
//! in flight, stops the turn, and requires the turn to be durably cancelled
//! while the target is still held; it releases the target only afterwards.
//! Before the fix the stop waited on the session lock the tool phase holds
//! across the call, so neither the ack nor the cancellation arrived until the
//! dispatch timeout.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::evidence_data::message_text;
use crate::fixtures::ScenarioFixture;

const ID: &str = "INT-039";
const MESSAGE: &str = "Take a snapshot.";
const CALL_ID: &str = "call-1";

pub(super) fn scenario() -> ScenarioFixture {
    let model = Model::scripted("fixture-model");
    let snapshot = ControlledFunction::new(
        "{{run_id}}::snapshot",
        "Take one integration fixture snapshot.",
    )
    .request_schema(json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "value": { "type": "string" } },
        "required": ["value"]
    }))
    .returns_text("snapshot")
    .hold_response();

    Scenario::new(
        ID,
        "stop-held-call",
        "Stopping a turn whose function call never returns cancels it without waiting for the call.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-039")
            .allow_function(&snapshot),
    )
    .function(snapshot.clone())
    .stop_held_call()
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact([snapshot.tool()]),
            )
            .respond(Response::function_call(
                CALL_ID,
                &snapshot,
                json!({ "value": "page" }),
                8,
                4,
            )),
    )
    .scenario_timeout_ms(60_000)
    .verify(verify)
    .build()
}

fn verify(run: &crate::evidence_data::RunEvidence) -> anyhow::Result<()> {
    anyhow::ensure!(
        run.control.pointer("/stop_response/stopping") == Some(&json!(true))
            && run
                .control
                .pointer("/status_while_held/status")
                .and_then(serde_json::Value::as_str)
                == Some("cancelled"),
        "the turn was not cancelled while its call was held: {}",
        run.control
    );
    anyhow::ensure!(
        run.status.get("status").and_then(serde_json::Value::as_str) == Some("cancelled"),
        "turn status must be cancelled: {}",
        run.status
    );
    run.expect_target_calls(1)?;

    let function_id = format!("{}::snapshot", run.run_id);
    let results = run.function_results(&function_id);
    anyhow::ensure!(
        results.len() == 1,
        "stopped call has {} closing function results, expected 1",
        results.len()
    );
    let result = results[0];
    anyhow::ensure!(
        result
            .get("function_call_id")
            .and_then(serde_json::Value::as_str)
            == Some(CALL_ID)
            && result.get("is_error").and_then(serde_json::Value::as_bool) == Some(true)
            && result
                .pointer("/details/error")
                .and_then(serde_json::Value::as_str)
                == Some("cancelled"),
        "stopped call must close as a cancelled error: {result}"
    );
    anyhow::ensure!(
        message_text(result).contains("stopped by the user"),
        "stopped call result lacks the explanation: {result}"
    );

    let transcript = serde_json::to_string(&run.transcript)?;
    anyhow::ensure!(
        transcript.contains("stopped by user"),
        "transcript is missing the durable stop notice"
    );
    anyhow::ensure!(
        run.generations_consumed == 1 && run.generations_total == 1,
        "{} of {} scripted generations consumed",
        run.generations_consumed,
        run.generations_total
    );
    run.expect_no_duplicate_messages()
}
