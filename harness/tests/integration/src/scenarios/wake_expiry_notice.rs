//! INT-017 — a wake whose lifecycle deadline passes UNFIRED wakes its owner
//! with the news, and the session un-parks.
//!
//! This pins the parked-forever hole found in the reactive discovery run: a
//! coordinator armed a one-shot state wake on a key nothing ever wrote and
//! slept next to a finished run indefinitely. Two defects compounded:
//!
//! * `expires_at` on a wake protected nothing — expiry was only consulted at
//!   claim time, so with no fire ever arriving the deadline never mattered;
//! * `session_expects_wake` counted exhausted bindings, so even an EXPIRED
//!   wake kept its session non-terminal forever.
//!
//! The fix under test: the expiry sweep retires the binding (delete-first, so
//! it can never race a real fire), injects a `[notification]` naming the
//! watch, the deadline, and the finality, and writes the same `trigger_fired`
//! record every delivery outcome writes. The woken turn then completes
//! TERMINAL — proven here by the run finishing at all, since the completion
//! waiter only counts terminal turns.
//!
//! Shape: the arm turn is a PARKED completion (`terminal: false` — the first
//! in the suite), declared via `parked_completions(1)`; the expiry wake is an
//! externally initiated turn with its own trace. The binding's `expires_at`
//! is a `{{now_plus_…ms}}` token resolved at run expansion, and the sweep
//! interval is shrunk via the harness env so expiry lands in seconds.

use serde_json::{json, Value};

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const SCOPE: &str = "e2e-012";
const KEY: &str = "never";
/// The harness's expiry-sweep cadence knob (`bindings::expiry` — the e2e
/// crate tests the BINARY, so the name is pinned here as a string and by the
/// harness's own `sweep_interval_parses_only_positive_ms` unit test).
const SWEEP_INTERVAL_ENV: &str = "III_HARNESS_EXPIRY_SWEEP_MS";

/// Comfortably after the arm turn completes (~2-4s from expansion including
/// stack boot), comfortably inside the 60s scenario deadline with the 500ms
/// sweep. The margin is deliberate: a slow machine must still finish arming
/// before the deadline passes, or the notification lands mid-turn and the
/// park never happens.
const EXPIRES_IN: &str = "{{now_plus_12000ms}}";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-017";
    const MESSAGE: &str = "Arm a doomed wake.";

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

    // A one-shot WAKE on a state key NOTHING will ever write — the discovery
    // run's exact shape — with the lifecycle deadline the fix makes real.
    let register_args = json!({
        "trigger_type": "state",
        "config": { "scope": SCOPE, "key": KEY },
        "once": true,
        "label": "doomed",
        "lifecycle": { "expires_at": EXPIRES_IN }
    });

    Scenario::new(
        ID,
        "wake-expiry-notice",
        "A once-wake whose lifecycle deadline passes unfired notifies its owner and the \
         parked session goes terminal.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-012")
            .allow_id(REGISTER)
            .allow_function(&record),
    )
    // The arm turn completes PARKED (armed unexpired wake ⇒ terminal: false);
    // only the expiry-woken turn is terminal.
    .terminal_turn_statuses(["completed", "completed"])
    .parked_completions(1)
    // The send's trace plus the externally initiated expiry wake's.
    .expect_traces(2)
    .harness_env(SWEEP_INTERVAL_ENV, "500")
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
    // The expiry-woken turn: a fresh externally initiated turn carrying the
    // wake-lost notification as its user message. Its prompt is NOT the arm
    // turn's — the sweep's engine-side unregister is a registry change, so
    // the staleness notice deterministically joins the prompt; pin the notice
    // instead of the sha (same drift INT-008 handles).
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex("registry changed during this conversation")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user" }),
                    ])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::text("expiry noted", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["armed and parked", "expiry noted"])?;

        // Exactly one notification, and it must carry everything the woken
        // session needs to act without a lookup: the watch, the zero-fire
        // fact, and the finality.
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
            "expected exactly 1 wake-lost notification, got {}: {notifications:?}",
            notifications.len()
        );
        let notice = &notifications[0];
        for needle in [
            "wake expired unfired",
            SCOPE,
            KEY,
            "0 fires",
            "Nothing else will wake this session",
        ] {
            anyhow::ensure!(
                notice.contains(needle),
                "notification must mention {needle:?}: {notice}"
            );
        }

        // The durable record: same custom_type as every delivery outcome,
        // with the expiry note, on the derived e_trigexpired_* entry id.
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
            "an expired binding must record retirement: {data}"
        );
        anyhow::ensure!(
            data.get("once").and_then(Value::as_bool) == Some(true),
            "the record must carry the wake's once flag: {data}"
        );
        let note = data.get("note").and_then(Value::as_str).unwrap_or_default();
        anyhow::ensure!(
            note.contains("expired unfired"),
            "the record's note must name the expiry: {data}"
        );
        anyhow::ensure!(
            data.get("scope").and_then(Value::as_str) == Some(SCOPE)
                && data.get("key").and_then(Value::as_str) == Some(KEY),
            "the record must carry the watch from the registration: {data}"
        );
        if let Some(id) = records[0]
            .get("entry_id")
            .or_else(|| records[0].get("id"))
            .and_then(Value::as_str)
        {
            anyhow::ensure!(
                id.starts_with("e_trigexpired_"),
                "expiry record id must be the derived e_trigexpired_* form, got {id}"
            );
        }

        // The un-park, from the durable status: with the binding retired the
        // session no longer expects a wake — the exact flag that kept the
        // discovery run's coordinator looking alive-but-parked forever.
        anyhow::ensure!(
            run.status.get("expects_wake").and_then(Value::as_bool) == Some(false),
            "the session must not expect a wake after the expiry notice: {}",
            run.status
        );
        anyhow::ensure!(
            run.status
                .get("armed_wakes")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty),
            "no armed wake may survive the sweep: {}",
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
    fn fixture_declares_the_park_and_the_wake_trace() {
        let fixture = scenario();
        fixture.validate().unwrap();
        // Two completions, ONE terminal: the arm turn parks.
        assert_eq!(fixture.expected_turn_statuses.len(), 2);
        assert_eq!(fixture.expected_terminal_turns, 1);
        // No probes — the wake is the sweep's, externally initiated.
        assert!(fixture.probe_actions.is_empty());
        assert_eq!(fixture.expected_traces(), 2);
        // The sweep knob rides the fixture env into the subject process.
        assert!(fixture
            .harness_env
            .iter()
            .any(|(k, v)| k == SWEEP_INTERVAL_ENV && v == "500"));
    }
}
