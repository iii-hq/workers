//! INT-041 — `harness::on-turn-overdue` reports a turn still running at its
//! check points, and stops once the turn completes.
//!
//! The seeded harness config sets both check-point intervals to 1.5 s
//! (`stack::config`). The controlled function holds its response, so the turn
//! stays `running` with the call `dispatched`; the runner observes the event for this
//! session, waits for two reports, releases the call, and waits two more
//! intervals after the turn completes. A watchdog session judges long turns
//! from exactly these reports (tech-specs/2026-11-watchdog).

use serde_json::{json, Value};

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const ID: &str = "INT-041";
const MESSAGE: &str = "Take a slow snapshot.";
const CALL_ID: &str = "call-1";
const REPORTS: usize = 2;
const CHECK_POINT_MS: i64 = 1_500;

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
    let arguments = json!({ "value": "page" });

    Scenario::new(
        ID,
        "turn-overdue",
        "A turn still running at its check points is reported overdue, and never after it completes.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-041")
            .allow_function(&snapshot),
    )
    .function(snapshot.clone())
    .turn_overdue(REPORTS)
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
                arguments.clone(),
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
                    .messages_exact([
                        Message::user(MESSAGE),
                        Message::function_call(CALL_ID, &snapshot, arguments, &model, 8, 4),
                        Message::function_result(CALL_ID, &snapshot, "snapshot"),
                    ])
                    .tools_exact([snapshot.tool()]),
            )
            .respond(Response::text("snapshot taken", 18, 2)),
    )
    .scenario_timeout_ms(60_000)
    .verify(verify)
    .build()
}

fn verify(run: &crate::evidence_data::RunEvidence) -> anyhow::Result<()> {
    let held = run
        .control
        .get("reports_while_held")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("no overdue reports in the control: {}", run.control))?;
    anyhow::ensure!(
        held.len() >= REPORTS,
        "{} overdue reports while the call was held, expected {REPORTS}",
        held.len()
    );
    let turn_id = run
        .status
        .get("turn_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("status has no turn_id: {}", run.status))?;
    let function_id = format!("{}::snapshot", run.run_id);
    let mut last_age = 0;
    for report in held {
        let age = report.get("age_ms").and_then(Value::as_i64).unwrap_or(0);
        anyhow::ensure!(
            report.get("turn_id").and_then(Value::as_str) == Some(turn_id)
                && report.get("depth") == Some(&json!(0))
                && report.get("status").and_then(Value::as_str) == Some("running")
                && age >= CHECK_POINT_MS
                && age > last_age,
            "overdue report does not describe the held root turn at a later check point: {report}"
        );
        let pending = report
            .get("pending_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        anyhow::ensure!(
            pending.iter().any(|call| {
                call.get("call_id").and_then(Value::as_str) == Some(CALL_ID)
                    && call.get("function_id").and_then(Value::as_str) == Some(&function_id)
                    && call.get("state").and_then(Value::as_str) == Some("dispatched")
            }),
            "overdue report does not name the held call: {report}"
        );
        last_age = age;
    }
    let at_completion = run
        .control
        .get("reports_at_completion")
        .and_then(Value::as_u64);
    let after_quiet = run
        .control
        .get("reports_after_quiet")
        .and_then(Value::as_array)
        .map(|reports| reports.len() as u64);
    anyhow::ensure!(
        at_completion.is_some() && at_completion == after_quiet,
        "a completed turn was reported overdue again: {} at completion, {:?} two check points later",
        at_completion.unwrap_or_default(),
        after_quiet
    );

    run.expect_assistant_texts(["snapshot taken"])?;
    run.expect_target_calls(1)?;
    anyhow::ensure!(
        run.generations_consumed == 2 && run.generations_total == 2,
        "{} of {} scripted generations consumed",
        run.generations_consumed,
        run.generations_total
    );
    run.expect_no_duplicate_messages()
}
