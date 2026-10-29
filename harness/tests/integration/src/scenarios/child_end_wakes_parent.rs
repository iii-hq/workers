//! INT-041 — a child that ends while its parent is parked on a wake wakes the
//! parent with the news, long before the wake's own deadline.
//!
//! This pins the 21-minute stall found in a deepwork run: an orchestrator armed
//! a state wake for its child's result, spawned the child and parked. The child
//! ended without ever writing that key (here: it wrote another scope), so
//! nothing ever fired the wake and the parent slept until the wake's lifecycle
//! deadline. The harness knew the child had ended and told nobody.
//!
//! The fix under test: the child's finalize injects a
//! `[notification] child ended: session <id> (turn <id>) status=<s>` into the
//! parent (deterministic entry id `e_childend_<child turn>`), which un-parks
//! it. The wake's deadline here is a full minute and is never reached.
//!
//! Shape: INT-005's parent (arm, spawn, park) with INT-017's un-park. The child
//! takes three generations against the parent's one, as in INT-005, so the
//! parent has parked before the child ends. The notice-woken turn tears the
//! doomed wake down, which is what makes it the terminal turn.

use serde_json::{json, Value};

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const UNREGISTER: &str = "engine::unregister_trigger";
const SPAWN: &str = "harness::spawn";
/// Far past the scenario deadline: the wake can only be un-parked by the notice.
const EXPIRES_IN_MS: u64 = 60_000;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-041";
    const MESSAGE: &str = "Delegate the report and wait for it.";

    let model = Model::scripted("fixture-model");

    // The parent waits for a key the child will never write.
    let register_args = json!({
        "trigger_type": "state",
        "config": { "scope": "{{run_id}}-result", "key": "report" },
        "once": true,
        "label": "child-result",
        "lifecycle": { "expires_in_ms": EXPIRES_IN_MS }
    });

    // The child writes somewhere else, then ends `completed`.
    let spawn_args = json!({
        "task": "Write state scope {{run_id}}-elsewhere key a value {\"n\":1}, then key b \
                 value {\"n\":2}. Then stop.",
        "session_id": "{{run_id}}-child",
        "options": { "functions": { "allow": ["state::set"] } }
    });

    Scenario::new(
        ID,
        "child-end-wakes-parent",
        "A child that ends completed without writing the key its parent's wake watches wakes \
         the parked parent with a child-ended notification, well before the wake's deadline.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-040")
            // Declared in sorted order, like INT-005: the harness renders the
            // policy prompt line sorted.
            .allow_id(REGISTER)
            .allow_id(UNREGISTER)
            .allow_id(SPAWN)
            .allow_id("state::set"),
    )
    // Parent and child turns are independently scheduled; match each request by
    // its strict contract instead of relying on queue arrival order.
    .match_any_dispatch()
    // The arm-and-spawn turn completes PARKED; the notice-woken turn is the
    // single terminal one, because it unregisters the wake.
    .terminal_turn_statuses(["completed", "completed"])
    .parked_completions(1)
    // The send's trace (parent and child) plus the notice-woken turn's.
    .expect_traces(2)
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
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
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-spawn",
                SPAWN,
                spawn_args,
                8,
                4,
            )),
    )
    // The child's three generations and the parent's post-spawn one may arrive
    // in either order; match-any dispatch keeps each strict.
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex("You are an iii agent")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-w1",
                "state::set",
                json!({ "scope": "{{run_id}}-elsewhere", "key": "a", "value": { "n": 1 } }),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(2)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-arm", "function_id": REGISTER }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-arm",
                                "is_error": false }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-spawn", "function_id": SPAWN }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-spawn",
                                "is_error": false }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("armed, spawned, parked", 10, 2)),
    )
    .generation(
        Generation::new(5)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex("You are an iii agent")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-w1" }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-w1",
                                "is_error": false }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-w2",
                "state::set",
                json!({ "scope": "{{run_id}}-elsewhere", "key": "b", "value": { "n": 2 } }),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(6)
            .expect(
                Request::new()
                    .turn_request_step(2)
                    .system_prompt_regex("You are an iii agent")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-w2" }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-w2",
                                "is_error": false }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("wrote the wrong scope", 10, 2)),
    )
    // The notice-woken parent turn: a fresh turn whose last user message is the
    // child-ended notification. It reads the subscription id out of the arm
    // call's result (a serve-time capture) and tears the doomed wake down.
    .generation(
        Generation::new(7)
            .capture("wake", "(sub_[0-9a-f]{32})")
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(".")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user" }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-unarm",
                UNREGISTER,
                json!({ "id": "[[cap:wake]]" }),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(8)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex(".")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-unarm", "function_id": UNREGISTER }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-unarm",
                                "is_error": false }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("child ended, wake removed", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["armed, spawned, parked", "child ended, wake removed"])?;

        // Exactly one notification reached the parent, and it is the child's
        // end: the wake never expired and nothing else woke the parent.
        let notices: Vec<(&str, String)> = run
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
                text.contains("[notification]").then_some((
                    item.get("entry_id").and_then(Value::as_str).unwrap_or_default(),
                    text,
                ))
            })
            .collect();
        anyhow::ensure!(
            notices.len() == 1,
            "expected exactly 1 notification (the child's end), got {}: {notices:?}",
            notices.len()
        );
        let (entry_id, text) = &notices[0];
        let prefix = format!(
            "[notification] child ended: session {}-child (turn ",
            run.run_id
        );
        anyhow::ensure!(
            text.starts_with(&prefix),
            "the notice must open with the line the kit parses ({prefix:?}): {text}"
        );
        let turn = text[prefix.len()..].split(')').next().unwrap_or_default();
        anyhow::ensure!(
            *entry_id == format!("e_childend_{turn}"),
            "the notice must sit on the deterministic entry e_childend_<child turn> \
             ({turn:?}), got {entry_id:?}"
        );
        anyhow::ensure!(
            text.contains(&format!("(turn {turn}) status=completed")),
            "the child ended completed: {text}"
        );

        // The un-park came from the notice, not from the wake giving up.
        for item in &run.transcript {
            let raw = serde_json::to_string(item).unwrap_or_default();
            anyhow::ensure!(
                !raw.contains("wake expired"),
                "the wake must not have reached its deadline: {raw}"
            );
        }
        anyhow::ensure!(
            run.status.get("expects_wake").and_then(Value::as_bool) == Some(false),
            "the parent must not expect a wake after tearing it down: {}",
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
    fn fixture_declares_the_park_the_child_and_the_notice_wake() {
        let fixture = scenario();
        fixture.validate().unwrap();
        // Two completions, ONE terminal: the arm turn parks.
        assert_eq!(fixture.expected_turn_statuses.len(), 2);
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.expected_traces(), 2);
        assert_eq!(
            fixture.script.dispatch,
            crate::types::script::RouterDispatchV1::MatchAny
        );
        // Eight generations: five parent (arm, spawn, park, unregister, close)
        // and three child. The floor's all-generations-consumed check is what
        // proves the child ran to its end.
        assert_eq!(fixture.script.generations.len(), 8);
        assert!(fixture.probe_actions.is_empty());
    }
}
