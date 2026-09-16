//! INT-030 — a one-shot state wake registered on a key that ALREADY holds a
//! value is delivered at registration, retires, and leaves nothing parked.
//!
//! The Linkly `link-build-e2e` postmortem (run `0491ffe3…`): the parent read
//! `state::get(scope, b)` → `null`, the sub-agent wrote `b`, the parent then
//! registered a once-wake on `scope/b`. The registration succeeded with the
//! advisory "state …/b ALREADY holds a value — state events do not replay",
//! the parent re-read `b` (`{ status: "ok" }`) but never removed the binding,
//! `fires` stayed `0`, `session_expects_wake` kept the session non-terminal,
//! and `harness::metrics` reported `complete: false` until the 900 s watchdog.
//!
//! The fix under test (`bindings::state_wake`): right after the trigger is
//! registered the harness re-reads the key and, when it holds a value,
//! delivers that value NOW as a synthetic `state:updated` event
//! (`replayed: true`) through the ordinary delivery hop — claim CAS,
//! `trigger_fired` record, once-retirement, the lot. The registration
//! response says so (`delivered: true` + a note), so the registrant never has
//! to infer it.
//!
//! The harness cannot distinguish "written before the preflight read" from
//! "written between the preflight read and the provider's activation": both
//! leave the key holding a value when the post-registration read runs, so
//! this one fixture pins both. Shape: the scripted model writes the key
//! itself (`state::set`, native tool), then registers the wake — all inside
//! ONE turn. The catch-up delivery lands while the turn is running, so the
//! wake parks as a queued row and the loop drains it into the NEXT step: the
//! model sees the `[notification]` in the same turn, no park, one terminal
//! turn, one trace. Nothing here is timing-dependent.

use serde_json::{json, Value};

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const STATE_SET: &str = "state::set";
const SCOPE: &str = "e2e-030";
const KEY: &str = "b";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-030";
    const MESSAGE: &str = "Write the key, then arm a wake on it.";

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

    // The sub-agent's write, made by the parent itself for determinism: the
    // key holds the value BEFORE the wake exists.
    let write_args = json!({
        "scope": SCOPE,
        "key": KEY,
        "value": { "status": "ok" }
    });
    // The Linkly parent's exact registration: a one-shot wake on scope/key.
    let register_args = json!({
        "trigger_type": "state",
        "config": { "scope": SCOPE, "key": KEY },
        "once": true,
        "label": "prewritten"
    });

    Scenario::new(
        ID,
        "state-wake-prewritten-key",
        "A one-shot state wake registered on a key that already holds a value is delivered at \
         registration from that value, retires, and parks nothing.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-030")
            .allow_id(REGISTER)
            .allow_id(STATE_SET)
            .allow_function(&record),
    )
    // ONE terminal turn: the caught-up wake is consumed inside it.
    .terminal_turn_statuses(["completed"])
    .function(record.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-write",
                STATE_SET,
                write_args,
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
                            { "type": "function_call", "id": "call-write", "function_id": STATE_SET }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-write",
                                "is_error": false }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-arm",
                REGISTER,
                register_args,
                8,
                4,
            )),
    )
    // The caught-up wake, drained into the very next step as a user message
    // AFTER the registration's function result: the model learns both that
    // the registration delivered and what it delivered, in one request.
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(2)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result", "function_call_id": "call-write" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-arm", "function_id": REGISTER }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-arm",
                                "is_error": false }),
                        json!({ "role": "user" }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("caught up", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["caught up"])?;

        // The registration response is DETERMINISTIC about what happened:
        // `delivered: true` and a note naming the replay, never the old
        // "does not replay" warning and never the "stays parked" advisory.
        let registration = function_result_text(run, "call-arm")
            .ok_or_else(|| anyhow::anyhow!("no function_result for call-arm in the transcript"))?;
        anyhow::ensure!(
            registration.contains("\"delivered\":true"),
            "the registration must report the catch-up delivery: {registration}"
        );
        for needle in ["ALREADY held a value", "delivered NOW", "replayed: true", "NOT parked"] {
            anyhow::ensure!(
                registration.contains(needle),
                "registration note must say {needle:?}: {registration}"
            );
        }
        for stale in ["do not replay", "stays parked"] {
            anyhow::ensure!(
                !registration.contains(stale),
                "registration note must not carry the pre-fix advisory {stale:?}: {registration}"
            );
        }

        // Exactly ONE notification, carrying the replayed event and the
        // value the key held.
        let notifications = notifications(run);
        anyhow::ensure!(
            notifications.len() == 1,
            "expected exactly 1 wake notification, got {}: {notifications:?}",
            notifications.len()
        );
        let notice = &notifications[0];
        for needle in [SCOPE, KEY, "\"replayed\":true", "\"status\":\"ok\""] {
            anyhow::ensure!(
                notice.contains(needle),
                "the wake must carry {needle:?}: {notice}"
            );
        }

        // Exactly ONE delivery record, and it consumed the once.
        let records = trigger_fired_records(run);
        anyhow::ensure!(
            records.len() == 1,
            "expected exactly 1 trigger_fired record, got {}: {records:?}",
            records.len()
        );
        let data = &records[0];
        anyhow::ensure!(
            data.get("outcome").and_then(Value::as_str) == Some("delivered"),
            "the record must be a delivery: {data}"
        );
        anyhow::ensure!(
            data.get("retired").and_then(Value::as_bool) == Some(true)
                && data.get("retirement_reason").and_then(Value::as_str) == Some("once_consumed"),
            "the catch-up delivery must retire the one-shot: {data}"
        );
        anyhow::ensure!(
            data.get("fires").and_then(Value::as_u64) == Some(1),
            "the catch-up is fire 1: {data}"
        );
        anyhow::ensure!(
            data.pointer("/payload/replayed").and_then(Value::as_bool) == Some(true),
            "the recorded payload must be marked as replayed: {data}"
        );
        anyhow::ensure!(
            data.get("scope").and_then(Value::as_str) == Some(SCOPE)
                && data.get("key").and_then(Value::as_str) == Some(KEY),
            "the record must carry the watch: {data}"
        );

        // Nothing parked — the exact flags the Linkly parent got stuck on.
        anyhow::ensure!(
            run.status.get("expects_wake").and_then(Value::as_bool) == Some(false),
            "the session must not expect a wake after the catch-up: {}",
            run.status
        );
        anyhow::ensure!(
            run.status
                .get("armed_wakes")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty),
            "no armed wake may survive the catch-up: {}",
            run.status
        );
        // …and the watchdog's own verdict, the one that timed out for 900 s.
        anyhow::ensure!(
            run.metrics.get("complete").and_then(Value::as_bool) == Some(true),
            "harness::metrics must report the tree complete: {}",
            run.metrics
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

/// The text a function result carried back to the model, by call id.
pub(super) fn function_result_text(
    run: &crate::evidence_data::RunEvidence,
    call_id: &str,
) -> Option<String> {
    run.transcript.iter().find_map(|item| {
        let msg = item.get("message")?;
        if msg.get("role").and_then(Value::as_str) != Some("function_result")
            || msg.get("function_call_id").and_then(Value::as_str) != Some(call_id)
        {
            return None;
        }
        Some(
            msg.get("content")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect(),
        )
    })
}

/// Every `[notification]` user message, in transcript order.
pub(super) fn notifications(run: &crate::evidence_data::RunEvidence) -> Vec<String> {
    run.transcript
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
        .collect()
}

/// The `data` of every `trigger_fired` record, in transcript order.
pub(super) fn trigger_fired_records(run: &crate::evidence_data::RunEvidence) -> Vec<Value> {
    run.transcript
        .iter()
        .filter_map(|item| {
            let custom = item.get("custom")?;
            (custom.get("custom_type").and_then(Value::as_str) == Some("trigger_fired"))
                .then(|| custom.get("data").cloned().unwrap_or(Value::Null))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_is_a_single_terminal_turn_with_no_probe() {
        let fixture = scenario();
        fixture.validate().unwrap();
        // The whole catch-up happens inside the arm turn: one completion,
        // terminal, no external stimulus.
        assert_eq!(fixture.expected_turn_statuses, vec!["completed"]);
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert!(fixture.probe_actions.is_empty());
        assert_eq!(fixture.expected_traces(), 1);
    }
}
