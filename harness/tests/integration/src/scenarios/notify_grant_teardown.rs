//! INT-016 — a notify subscription fires into the owning session, filesystem
//! grants round-trip, the shared budget ledger reconciles, and
//! `harness::teardown` sweeps the surviving binding.
//!
//! One run covers four cold seams at once:
//! - `subscriptions/notify_agent.rs`: the state fire lands as a durable
//!   `trigger_fired` custom entry plus an injected `[notification: …]` user
//!   message that seeds the second tracked turn. Both bindings are standing
//!   (`once: false`): an armed one-shot wake would park the session and keep
//!   the registering turn non-terminal, stalling the probe boundary.
//! - `functions/filesystem.rs` + `filesystem_grants.rs`: grant → grants →
//!   revoke round-trip through recorded probe responses.
//! - `budget.rs` happy path: a generous frozen budget admits every
//!   generation; after both turns the ledger holds the exact reconciled
//!   usage with no outstanding reservation.
//! - `teardown.rs` + `subscriptions/reconcile.rs::sweep_owner`: the second,
//!   never-fired `once: false` subscription survives until `harness::teardown`
//!   removes it.

use serde_json::{json, Value};

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const SCOPE: &str = "integration-016";
const FIRE_KEY: &str = "fire";
const IDLE_KEY: &str = "idle";
const BUDGET_TOKENS: u64 = 200_000;
/// Scripted usage across the three generations: (8+4) + (10+2) + (12+3).
const EXPECTED_USED_TOKENS: u64 = 39;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-016";
    const MESSAGE: &str = "Arm two standing state notifications.";

    let subscribe_fire = json!({
        "trigger_type": "state",
        "config": { "scope": SCOPE, "key": FIRE_KEY },
        "label": "fire-key changed",
        "once": false
    });
    let subscribe_idle = json!({
        "trigger_type": "state",
        "config": { "scope": SCOPE, "key": IDLE_KEY },
        "label": "idle-key changed",
        "once": false
    });

    Scenario::new(
        ID,
        "notify-grant-teardown",
        "A notify subscription fires an injected notification turn; filesystem grants \
         round-trip; the budget ledger reconciles; teardown sweeps the standing binding.",
        ScenarioDriver::Direct,
        Model::scripted("fixture-model"),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-016")
            .allow_id(REGISTER)
            .max_total_tokens(BUDGET_TOKENS),
    )
    .terminal_turns(2)
    // Two trees: the registering turn and the injected-notification turn.
    // Probe calls here are plain RPCs — none seeds an extra trace.
    .expect_traces(2)
    // Turn 1 done: trip the once subscription's key. The notify fire injects
    // the user message that seeds turn 2.
    .probe_after(
        1,
        "state::set",
        json!({ "scope": SCOPE, "key": FIRE_KEY, "value": { "seq": 1 } }),
    )
    // Turn 2 done: grants round-trip, ledger, and teardown — all recorded.
    .probe_after(
        2,
        "harness::filesystem::grant",
        json!({ "session_id": "{{session_id}}", "root": "/tmp/int-016" }),
    )
    .probe_after(
        2,
        "harness::filesystem::grants",
        json!({ "session_id": "{{session_id}}" }),
    )
    .probe_after(
        2,
        "harness::filesystem::revoke",
        json!({ "session_id": "{{session_id}}", "root": "/tmp/int-016" }),
    )
    .probe_after(
        2,
        "state::get",
        json!({ "scope": "harness_budget", "key": "{{session_id}}" }),
    )
    .probe_after(
        2,
        "harness::teardown",
        json!({ "root_session_id": "{{session_id}}" }),
    )
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact_after_controls([REGISTER], Vec::new()),
            )
            .respond(Response::function_calls_raw(
                vec![
                    ("call-sub-fire", REGISTER, subscribe_fire),
                    ("call-sub-idle", REGISTER, subscribe_idle),
                ],
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
                            { "type": "function_call", "id": "call-sub-fire" },
                            { "type": "function_call", "id": "call-sub-idle" }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-sub-fire",
                                "is_error": false }),
                        json!({ "role": "function_result", "function_call_id": "call-sub-idle",
                                "is_error": false }),
                    ])
                    .tools_exact_after_controls([REGISTER], Vec::new()),
            )
            .respond(Response::streamed_text("armed", ["armed"], 10, 2)),
    )
    // The injected-notification turn: a fresh turn in the SAME session whose
    // user message is the notify text.
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_exact_after_controls([REGISTER], Vec::new()),
            )
            .respond(Response::streamed_text("acknowledged", ["acknowledged"], 12, 3)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["armed", "acknowledged"])?;

        // The fire left a durable trigger_fired entry for the retired once-sub.
        let fired = run
            .transcript
            .iter()
            .filter_map(|item| {
                let custom_type = item
                    .pointer("/custom/custom_type")
                    .and_then(Value::as_str);
                (custom_type == Some("trigger_fired"))
                    .then(|| item.pointer("/custom/data"))
                    .flatten()
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(
            fired.len() == 1,
            "expected one trigger_fired entry, found {}",
            fired.len()
        );
        anyhow::ensure!(
            fired[0].get("once") == Some(&Value::Bool(false))
                && fired[0].get("retired") == Some(&Value::Bool(false)),
            "a standing subscription must not retire on fire: {}",
            fired[0]
        );

        // The injected notification message carries the label.
        let injected = run
            .transcript
            .iter()
            .filter_map(|item| item.get("message"))
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
            .map(crate::evidence_data::message_text)
            .find(|text| text.starts_with("[notification:"));
        anyhow::ensure!(
            injected
                .as_deref()
                .is_some_and(|text| text.contains("fire-key changed")),
            "injected notification message missing or unlabeled: {injected:?}"
        );

        let roots_of = |response: &Value| -> Vec<String> {
            response
                .get("roots")
                .and_then(Value::as_array)
                .map(|roots| {
                    roots
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        run.expect_probe_response(1, "harness::filesystem::grant", |response| {
            anyhow::ensure!(
                roots_of(response) == ["/tmp/int-016"],
                "grant did not persist the root: {response}"
            );
            Ok(())
        })?;
        run.expect_probe_response(2, "harness::filesystem::grants", |response| {
            anyhow::ensure!(
                roots_of(response) == ["/tmp/int-016"],
                "grants did not list the root: {response}"
            );
            Ok(())
        })?;
        run.expect_probe_response(3, "harness::filesystem::revoke", |response| {
            anyhow::ensure!(
                roots_of(response).is_empty(),
                "revoke did not drop the root: {response}"
            );
            Ok(())
        })?;
        run.expect_probe_response(4, "state::get", |response| {
            let ledger = response.get("value").unwrap_or(response);
            anyhow::ensure!(
                ledger.get("max_total_tokens").and_then(Value::as_u64) == Some(BUDGET_TOKENS),
                "ledger max_total_tokens != {BUDGET_TOKENS}: {ledger}"
            );
            anyhow::ensure!(
                ledger.get("used_tokens").and_then(Value::as_u64)
                    == Some(EXPECTED_USED_TOKENS),
                "ledger used_tokens != {EXPECTED_USED_TOKENS}: {ledger}"
            );
            anyhow::ensure!(
                ledger.get("reserved_tokens").and_then(Value::as_u64) == Some(0),
                "no reservation may remain after terminal turns: {ledger}"
            );
            Ok(())
        })?;
        run.expect_probe_response(5, "harness::teardown", |response| {
            anyhow::ensure!(
                response.get("removed").and_then(Value::as_u64) >= Some(2),
                "teardown must sweep both standing subscriptions: {response}"
            );
            Ok(())
        })?;
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_declares_two_turns_and_six_probe_actions() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.probe_actions.len(), 6);
        assert_eq!(fixture.expected_traces(), 2);
        assert_eq!(fixture.script.generations.len(), 3);
    }
}
