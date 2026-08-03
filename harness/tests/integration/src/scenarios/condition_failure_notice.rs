//! INT-019 — a binding whose condition FAILS to evaluate must wake its owner
//! with the news, not starve silently.
//!
//! The bug class this pins: an agent arms a wake for a rendezvous and gates
//! it with a condition whose call errors on every fire (a barrier pointed at
//! a pointer the event does not carry, a non-condition function wired as a
//! condition). Each fire is skipped with a `condition-error` record — but the
//! record lands in a transcript the PARKED owner never reads, so the session
//! sleeps forever next to a binding that can never deliver. Three live
//! receiving-op runs died exactly this way.
//!
//! The fixture wires the failure deterministically: the condition is the
//! suite's controlled function, whose fixed reply is not a decision shape, so
//! every evaluation is `undecipherable condition result` → gate
//! `condition-error`. The probe inserts one row; the fire is skipped; the
//! owner must be WOKEN by the `[notification] … NOT delivered …` notice and
//! the binding must survive (skips never consume the lifecycle).
//!
//! Same standing + `max_fires: 1` shape as INT-014, for the same reason: the
//! probe boundary needs a terminal first turn.

use serde_json::{json, Value};

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const TABLE: &str = "courier_done";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-019";
    const MESSAGE: &str = "Arm the courier_done watch with a completion checker.";

    let model = Model::scripted("fixture-model");
    // The "checker" an agent might plausibly wire as a condition: an ordinary
    // function whose answer is not a decision. Every condition call on it
    // fails to parse — the deterministic stand-in for the live BARRIER_ERROR.
    let checker = ControlledFunction::new(
        "{{run_id}}::check",
        "Completion checker for the courier_done watch.",
    )
    .request_schema(json!({ "type": "object" }))
    .returns_text("okay");

    let register_args = json!({
        "trigger_type": "database::row-changed",
        "config": { "db": "primary", "table": TABLE },
        "once": false,
        "lifecycle": { "max_fires": 1 },
        "conditions": [{ "function_id": checker.id(), "config": {} }],
    });

    Scenario::new(
        ID,
        "condition-failure-notice",
        "A binding whose condition errors on a fire wakes its owner with an actionable notice \
         instead of starving silently; the binding stays armed.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-019")
            .allow_id(REGISTER)
            .allow_function(&checker),
    )
    .terminal_turn_statuses(["completed", "completed"])
    // One transaction: DDL + the INSERT. Only the INSERT emits a row event;
    // it reaches the binding, the condition call fails, and the notice — not
    // a delivery — is what wakes the idle session.
    .probe_after(
        1,
        "database::executeBatch",
        json!({
            "db": "primary",
            "statements": [
                "CREATE TABLE IF NOT EXISTS courier_done (supplier TEXT, done_at INTEGER)",
                "INSERT INTO courier_done (supplier, done_at) VALUES ('acme', 1)"
            ]
        }),
    )
    .function(checker.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact_after_controls([REGISTER], [checker.tool()]),
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
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-arm", "function_id": REGISTER }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-arm",
                                "is_error": false }),
                    ])
                    .tools_exact_after_controls([REGISTER], [checker.tool()]),
            )
            .respond(Response::text("armed: watching courier_done", 10, 2)),
    )
    // The woken turn. Without the notice this generation is never requested
    // and the run times out awaiting its second terminal — the bite of this
    // fixture.
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(".")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact_after_controls([REGISTER], [checker.tool()]),
            )
            .respond(Response::text("condition failure acknowledged", 12, 3)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["armed: watching courier_done", "condition failure acknowledged"])?;
        // The condition was genuinely evaluated (the controlled target saw
        // the call) — the skip came from its unusable answer, not from the
        // condition never running.
        run.expect_function_calls("check", 1)?;

        let notification = run
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
            .next()
            .ok_or_else(|| anyhow::anyhow!("no [notification] notice in the transcript"))?;
        for needle in [
            "NOT delivered",
            "undecipherable condition result",
            "stays armed",
            "read the watched state once",
        ] {
            anyhow::ensure!(
                notification.contains(needle),
                "the notice must carry {needle:?}: {notification}"
            );
        }

        // The timeline record still exists alongside the notice, and the skip
        // did NOT consume the lifecycle: the binding survives its failed fire.
        let skip_record = run
            .transcript
            .iter()
            .filter_map(|item| item.get("custom"))
            .find(|c| {
                c.get("custom_type").and_then(Value::as_str) == Some("trigger_fired")
                    && c.pointer("/data/note")
                        .and_then(Value::as_str)
                        .is_some_and(|note| note.contains("undecipherable condition result"))
            })
            .ok_or_else(|| anyhow::anyhow!("no condition-error skip record in the transcript"))?;
        anyhow::ensure!(
            skip_record.pointer("/data/retired").and_then(Value::as_bool) == Some(false),
            "a condition failure must leave the binding armed: {skip_record}"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_gates_the_wake_on_the_notice_not_a_delivery() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.probe_actions.len(), 1);
        assert_eq!(fixture.probe_actions[0].after_turns, 1);
        assert_eq!(fixture.script.generations.len(), 3);

        // The registration wires the controlled function as the condition —
        // the whole point of the fixture.
        let register = serde_json::to_string(&fixture.script.generations[0].frames).unwrap();
        assert!(register.contains("\"conditions\""), "{register}");
        assert!(register.contains("{{run_id}}::check"), "{register}");
    }
}
