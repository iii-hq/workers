//! INT-016 — a standing wake binding delivers EVERY fire: each one as a
//! notification message AND a `trigger_fired` record, on distinct entry ids.
//!
//! This pins the burst-loss bug chain found live with `database::row-changed`
//! (one transaction committing three writes → three claims, two records, one
//! notification). Two defects compounded:
//!
//! * the fire claim was a `get` then a `put`, so simultaneous fires shared an
//!   ordinal (now a compare-and-set, pinned by the 40-way unit race);
//! * the wake message and its delivery record shared ONE entry id, and
//!   session-manager is idempotent on entry ids, so exactly one of the two
//!   appends survived each fire — the record on an idle session, the WAKE
//!   when a turn was running.
//!
//! The second defect is what this scenario reproduces, deterministically: on
//! an idle session the wake always appends first, so with shared ids the
//! record is ALWAYS swallowed — one fire suffices, no burst or race needed.
//! Two turn-gated fires additionally pin the ordinal chain: if fire 2 reused
//! fire 1's ordinal (a claim regression), its entry ids would dedupe against
//! fire 1's and the second notification would vanish.
//!
//! Everything is turn-gated (each probe waits for the previous terminal
//! turn), so every fire lands on an idle session and the turn and trace
//! counts are exact — no timing window anywhere.

use serde_json::{json, Value};

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const SCOPE: &str = "e2e-011";
const KEY: &str = "tick";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-016";
    const MESSAGE: &str = "Arm a standing wake.";

    let model = Model::scripted("fixture-model");
    // Never called — it exists so the send policy exposes a native tool and
    // every generation can pin `tools_exact` the way the sibling scenarios do.
    let record = ControlledFunction::new("{{run_id}}::record", "Record one value.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        }))
        .returns_text("recorded");

    // A NOTIFY binding (no function_id): each fire is injected into THIS
    // session as a `[notification]` message. `once: false` is the subject —
    // the binding must survive fire 1 and deliver fire 2 on the next ordinal.
    let register_args = json!({
        "trigger_type": "state",
        "config": { "scope": SCOPE, "key": KEY },
        "once": false,
        "label": "standing"
    });

    Scenario::new(
        ID,
        "standing-wake-delivery",
        "A standing notify binding delivers every fire as a notification AND a trigger_fired \
         record, on distinct entry ids.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-011")
            .allow_id(REGISTER)
            .allow_function(&record),
    )
    // Arm turn + one woken turn per fire.
    .terminal_turn_statuses(["completed", "completed", "completed"])
    // Fire 1 only after the arm turn is terminal; fire 2 only after fire 1's
    // woken turn is terminal. Every fire hits an idle session.
    .probe_after(
        1,
        "state::set",
        json!({ "scope": SCOPE, "key": KEY, "value": { "n": 1 } }),
    )
    .probe_after(
        2,
        "state::set",
        json!({ "scope": SCOPE, "key": KEY, "value": { "n": 2 } }),
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
            .respond(Response::text("armed", 10, 2)),
    )
    // Fire 1's woken turn. The notification text carries the state event
    // (nondeterministic worker id inside), so the request match stays
    // role-shaped; the verify pins the contents from the transcript.
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user" }),
                    ])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::text("wake one", 10, 2)),
    )
    // Fire 2's woken turn — the standing binding delivering again.
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user" }),
                    ])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::text("wake two", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["armed", "wake one", "wake two"])?;

        // Every fire's notification message, in fire order, carrying its own
        // event. A shared wake/record id loses one of these whenever the
        // record appends first; a reused ordinal dedupes the second outright.
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
            notifications.len() == 2,
            "expected 2 notification messages, got {}: {notifications:?}",
            notifications.len()
        );
        anyhow::ensure!(
            notifications[0].contains("\"n\":1") && notifications[1].contains("\"n\":2"),
            "notifications must carry their own events in fire order: {notifications:?}"
        );

        // Every fire's durable delivery record. Pre-fix, an idle-session fire
        // ALWAYS lost this to the wake's identical entry id.
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
            records.len() == 2,
            "expected 2 trigger_fired records, got {}",
            records.len()
        );
        // Both fires belong to the ONE standing binding, and it retired for
        // neither (`once: false`).
        for record in &records {
            let data = record
                .get("custom")
                .and_then(|c| c.get("data"))
                .cloned()
                .unwrap_or(Value::Null);
            anyhow::ensure!(
                data.get("retired").and_then(Value::as_bool) == Some(false),
                "a standing binding must not retire on a fire: {data}"
            );
        }
        // The record ids are the wake ids' derived twins, never the wake ids
        // themselves — the collision under test. Entry-id field name is the
        // session-manager's; tolerate either spelling but demand distinctness
        // when present.
        let ids: Vec<&str> = records
            .iter()
            .filter_map(|item| {
                item.get("entry_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
            })
            .collect();
        if ids.len() == 2 {
            anyhow::ensure!(
                ids[0] != ids[1],
                "the two delivery records collapsed onto one entry id: {ids:?}"
            );
            for id in ids {
                anyhow::ensure!(
                    id.starts_with("e_trigfired_"),
                    "delivery record id must be the derived e_trigfired_* form, got {id}"
                );
            }
        }
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_gates_each_fire_on_the_previous_terminal_turn() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 3);
        assert_eq!(fixture.probe_actions.len(), 2);
        assert_eq!(fixture.probe_actions[0].after_turns, 1);
        assert_eq!(fixture.probe_actions[1].after_turns, 2);
        // Turn-gated only: no probe races a running turn, so the turn and
        // trace floors stay exact.
        assert!(fixture
            .probe_actions
            .iter()
            .all(|p| p.after_target_calls.is_none()));
        // One send + one externally-initiated wake per probe.
        assert_eq!(fixture.expected_traces(), 3);
    }
}
