//! INT-014 — the database medium drives the SAME generic wake path state
//! does: a `database::row-changed` binding notifies its owner when a row
//! lands, with no database-specific anything in delivery.
//!
//! Together with INT-005/INT-006 (state) and INT-013 (timer) this pins the
//! medium-agnostic claim: the parent picks the shared mechanism — here a
//! SQLite table — registers ONE wake on its change stream, and the identical
//! deliver → notify hop wakes it. The stack runs the real `database` worker
//! (see `stack/config.rs`); the probe's `executeBatch` creates the table and
//! inserts the row, and only the INSERT emits (row events are op-classified,
//! not CDC).
//!
//! Same standing + `max_fires: 1` shape as INT-006, for the same reason: the
//! probe boundary needs a terminal first turn.

use serde_json::{json, Value};

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const TABLE: &str = "items";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-014";
    const MESSAGE: &str = "Watch the items table.";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new("{{run_id}}::record", "Record one value.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        }))
        .returns_text("recorded");

    let register_args = json!({
        "trigger_type": "database::row-changed",
        "config": { "db": "primary", "table": TABLE },
        "once": false,
        "lifecycle": { "max_fires": 1 }
    });

    Scenario::new(
        ID,
        "database-row-wake",
        "A database::row-changed wake notifies the owner through the same generic delivery \
         path the state medium uses.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-014")
            .allow_id(REGISTER)
            .allow_function(&record),
    )
    .terminal_turn_statuses(["completed", "completed"])
    // One transaction: DDL + the INSERT. The row event flushes post-commit,
    // fires the binding, and wakes the idle session.
    .probe_after(
        1,
        "database::executeBatch",
        json!({
            "db": "primary",
            "statements": [
                "CREATE TABLE IF NOT EXISTS items (id INTEGER PRIMARY KEY, name TEXT)",
                "INSERT INTO items (name) VALUES ('widget')"
            ]
        }),
    )
    .function(record.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
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
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::text("watching", 10, 2)),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(".")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::function_call(
                "call-record",
                &record,
                json!({ "value": "row-arrived" }),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex(".")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact_after_controls([REGISTER], [record.tool()]),
            )
            .respond(Response::text("row recorded", 12, 3)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["watching", "row recorded"])?;
        run.expect_function_calls("record", 1)?;

        // The notification carries the database worker's row event — the
        // table name and the classified op — proving the event payload flowed
        // through delivery untouched.
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
            .ok_or_else(|| anyhow::anyhow!("no [notification] wake in the transcript"))?;
        anyhow::ensure!(
            notification.contains(TABLE),
            "the wake must name the changed table: {notification}"
        );
        anyhow::ensure!(
            notification.contains("affected_rows"),
            "the wake must carry the row event: {notification}"
        );

        let retired = run.transcript.iter().any(|item| {
            item.get("custom")
                .and_then(|c| c.get("custom_type"))
                .and_then(Value::as_str)
                == Some("trigger_fired")
                && item
                    .get("custom")
                    .and_then(|c| c.get("data"))
                    .and_then(|d| d.get("retired"))
                    .and_then(Value::as_bool)
                    == Some(true)
        });
        anyhow::ensure!(
            retired,
            "the max_fires bound must retire the binding on delivery"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_mirrors_the_state_wake_shape_over_the_database_medium() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.probe_actions.len(), 1);
        assert_eq!(fixture.probe_actions[0].after_turns, 1);
    }
}
