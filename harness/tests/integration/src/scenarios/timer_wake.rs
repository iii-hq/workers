//! INT-013 — the `timer` trigger type: a one-shot relative deadline that
//! parks its session and fires exactly on time.
//!
//! The primitive exists because every live run that needed "tell me at T
//! that it didn't happen" reached for cron and fumbled the encoding —
//! boundary expressions fire early, the recurring default made deadlines
//! immortal, and discovery run 3 re-rolled an unbounded cron three times
//! rather than apply `once: true`. Here the whole deadline is ONE
//! registration: `{ "in_ms": 6000 }`, resolved to an absolute `at` at
//! registration, armed by the harness's own engine-registered provider,
//! delivered through the ordinary binding hop (claim → wake → record →
//! retire).
//!
//! Shape mirrors INT-012: the arm turn is a PARKED completion, the fire is an
//! externally initiated wake turn with its own trace. No sweep-interval env
//! and no `{{now_plus_…ms}}` token — `in_ms` is run-relative by nature, which
//! is the point of the feature.

use serde_json::{json, Value};

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-013";
    const MESSAGE: &str = "Arm a deadline.";

    let model = Model::scripted("fixture-model");
    // Never called — exposes a native tool so every generation pins
    // `tools_exact` the way the sibling scenarios do.
    let record = ControlledFunction::new("{{run_id}}::record", "Record one value.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        }))
        .returns_text("recorded");

    // Six seconds: comfortably after the scripted arm turn completes (~2s),
    // comfortably inside the scenario deadline.
    let register_args = json!({
        "trigger_type": "timer",
        "config": { "in_ms": 6000 },
        "label": "deadline"
    });

    Scenario::new(
        ID,
        "timer-wake",
        "A one-shot timer registration parks the session and wakes it exactly once when the \
         deadline passes.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-013")
            .allow_id(REGISTER)
            .allow_function(&record),
    )
    // The arm turn completes PARKED (a timer notify is an armed once-wake);
    // only the fired turn is terminal.
    .terminal_turn_statuses(["completed", "completed"])
    .parked_completions(1)
    .expect_traces(2)
    .function(record.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-arm",
                REGISTER,
                register_args,
                8,
                4,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-arm", "function_id": REGISTER }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-arm",
                                "is_error": false }),
                    ])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::text("armed and parked", 10, 2)),
    )
    // The fired turn. Retirement (engine unregister) races the woken turn's
    // start, so the registry-staleness prompt note is not deterministic here
    // — match the prompt loosely, the INT-008 way.
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(".")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user" }),
                    ])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::text("deadline hit", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["armed and parked", "deadline hit"])?;

        // Exactly one notification, carrying the timer event.
        let notifications: Vec<String> = run
            .transcript
            .iter()
            .filter_map(|item| {
                let msg = item.get("message")?;
                if msg.get("role").and_then(Value::as_str) != Some("user") {
                    return None;
                }
                let text: String = msg
                    .get("content")
                    .and_then(Value::as_array)?
                    .iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect();
                text.contains("[notification]").then_some(text)
            })
            .collect();
        anyhow::ensure!(
            notifications.len() == 1,
            "expected exactly 1 timer notification, got {}: {notifications:?}",
            notifications.len()
        );
        let notice = &notifications[0];
        for needle in ["\"trigger\":\"timer\"", "scheduled_at", "actual_at"] {
            anyhow::ensure!(
                notice.contains(needle),
                "notification must carry the timer event field {needle:?}: {notice}"
            );
        }

        // One delivered-fire record, retired by its own once lifecycle.
        let records: Vec<&Value> = run
            .transcript
            .iter()
            .filter(|item| {
                item.get("custom")
                    .and_then(|c| c.get("custom_type"))
                    .and_then(Value::as_str)
                    == Some("trigger_fired")
            })
            .collect();
        anyhow::ensure!(
            records.len() == 1,
            "expected exactly 1 trigger_fired record, got {}",
            records.len()
        );
        let data = records[0]
            .get("custom")
            .and_then(|c| c.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        anyhow::ensure!(
            data.get("retired").and_then(Value::as_bool) == Some(true),
            "a fired timer must retire: {data}"
        );
        anyhow::ensure!(
            data.get("once").and_then(Value::as_bool) == Some(true),
            "a timer binding is once by definition: {data}"
        );

        // The park resolved: nothing armed, nothing expected.
        anyhow::ensure!(
            run.status.get("expects_wake").and_then(Value::as_bool) == Some(false),
            "the session must not expect a wake after the timer fired: {}",
            run.status
        );
        anyhow::ensure!(
            run.status
                .get("armed_wakes")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty),
            "no armed wake may survive the fire: {}",
            run.status
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_declares_the_park_and_the_fire_trace() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_turn_statuses.len(), 2);
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert!(fixture.probe_actions.is_empty());
        assert_eq!(fixture.expected_traces(), 2);
        // The deadline is RELATIVE — no expansion token, no sweep knob; the
        // provider itself fires it. That absence is the feature.
        assert!(fixture.harness_env.is_empty());
    }
}
