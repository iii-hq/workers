//! INT-031 — the ordinary ordering, pinned next to INT-030: a one-shot state
//! wake registered while its key is ABSENT delivers nothing at registration,
//! parks its owner, and is woken exactly once by the later write.
//!
//! This is the Linkly topology done right — parent arms, THEN the writer
//! writes — and it guards the catch-up from over-firing: the registration-
//! time read finds `null` and must not deliver; the sweep's re-check (shrunk
//! to 500 ms here so it actually runs beside the live fire) finds `null`
//! until the write lands and must not deliver twice after it. Whichever of
//! the live fire and a sweep catch-up reaches the claim first delivers; the
//! other loses the CAS and records at most a `skipped` attempt, never a
//! second notification.
//!
//! Shape: the arm turn completes PARKED (`terminal: false`); the probe is
//! the external writer and fires `state::set` only AFTER that parked
//! completion (`probe_after_completion` — a terminal-only boundary can never
//! come, and a leaf writing on its own schedule landed mid-turn as steering
//! instead of a wake). The woken turn is the single terminal one.

use serde_json::{json, Value};

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::state_wake_prewritten_key::{
    function_result_text, notifications, trigger_fired_records,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const SCOPE: &str = "e2e-031";
const KEY: &str = "b";
/// `bindings::expiry`'s cadence knob, pinned as a string the INT-017 way.
const SWEEP_INTERVAL_ENV: &str = "III_HARNESS_EXPIRY_SWEEP_MS";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-031";
    const MESSAGE: &str = "Arm a wake on b before anyone writes it.";

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

    // The Linkly parent's registration, made BEFORE the writer runs.
    let register_args = json!({
        "trigger_type": "state",
        "config": { "scope": SCOPE, "key": KEY },
        "once": true,
        "label": "armed-first"
    });

    Scenario::new(
        ID,
        "state-wake-armed-before-write",
        "A one-shot state wake armed while its key is absent delivers nothing at registration, \
         parks, and is woken exactly once by the later write.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-031")
            .allow_id(REGISTER)
            .allow_function(&record),
    )
    // The arm turn completes PARKED; the woken turn is terminal.
    .terminal_turn_statuses(["completed", "completed"])
    .parked_completions(1)
    .expect_traces(2)
    .harness_env(SWEEP_INTERVAL_ENV, "500")
    // The external writer: the sub-agent's `state::set`, fired by the probe
    // once the arm turn has PARKED — the write lands on an idle, parked
    // session, exactly the ordering the Linkly parent intended.
    .probe_after_completion(
        1,
        "state::set",
        json!({ "scope": SCOPE, "key": KEY, "value": { "status": "ok" } }),
    )
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
    // The woken turn. Retirement (engine unregister) races the turn start,
    // so match the prompt loosely (the INT-013 way).
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
            .respond(Response::text("woken by the write", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["armed and parked", "woken by the write"])?;

        // At registration the key was absent: no delivery, the ordinary
        // "stays parked" advisory, and NO `delivered` flag.
        let registration = function_result_text(run, "call-arm")
            .ok_or_else(|| anyhow::anyhow!("no function_result for call-arm in the transcript"))?;
        anyhow::ensure!(
            !registration.contains("\"delivered\":true") && !registration.contains("delivered NOW"),
            "an absent key must not be reported as delivered: {registration}"
        );
        anyhow::ensure!(
            registration.contains("stays parked"),
            "the armed-wake advisory still applies to an absent key: {registration}"
        );

        // Exactly ONE notification — the live write, never a replay.
        let notifications = notifications(run);
        anyhow::ensure!(
            notifications.len() == 1,
            "expected exactly 1 wake notification, got {}: {notifications:?}",
            notifications.len()
        );
        let notice = &notifications[0];
        anyhow::ensure!(
            notice.contains(SCOPE) && notice.contains(KEY) && notice.contains("\"status\":\"ok\""),
            "the wake must carry the written value: {notice}"
        );
        anyhow::ensure!(
            !notice.contains("\"replayed\":true"),
            "a live write is not a replay: {notice}"
        );

        // Exactly ONE delivery. A sweep catch-up that lost the claim to the
        // live fire (or vice versa) may leave a `skipped` attempt behind —
        // that is the CAS working — but never a second delivery.
        let records = trigger_fired_records(run);
        let delivered: Vec<&Value> = records
            .iter()
            .filter(|data| data.get("outcome").and_then(Value::as_str) == Some("delivered"))
            .collect();
        anyhow::ensure!(
            delivered.len() == 1,
            "expected exactly 1 delivered record, got {}: {records:?}",
            delivered.len()
        );
        anyhow::ensure!(
            records.iter().all(|data| matches!(
                data.get("outcome").and_then(Value::as_str),
                Some("delivered" | "skipped")
            )),
            "only the delivery and a lost-claim skip may be recorded: {records:?}"
        );
        anyhow::ensure!(
            delivered[0].get("retired").and_then(Value::as_bool) == Some(true)
                && delivered[0]
                    .pointer("/payload/replayed")
                    .is_none_or(Value::is_null),
            "the live delivery consumed the once and was not a replay: {}",
            delivered[0]
        );

        anyhow::ensure!(
            run.status.get("expects_wake").and_then(Value::as_bool) == Some(false),
            "the woken session must not expect a wake: {}",
            run.status
        );
        anyhow::ensure!(
            run.metrics.get("complete").and_then(Value::as_bool) == Some(true),
            "harness::metrics must report the tree complete once woken: {}",
            run.metrics
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_parks_the_arm_turn_and_writes_after_the_park() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_turn_statuses.len(), 2);
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.expected_traces(), 2);
        // The writer fires at the PARKED completion, never a terminal one.
        assert_eq!(fixture.probe_actions.len(), 1);
        assert_eq!(fixture.probe_actions[0].after_turns, 1);
        assert!(fixture.probe_actions[0].count_parked);
        assert!(fixture
            .harness_env
            .iter()
            .any(|(k, v)| k == SWEEP_INTERVAL_ENV && v == "500"));
    }
}
