//! INT-005 — the parent-owned control plane, end to end over the state
//! medium: the top-level agent registers a barrier-gated wake, DIRECTLY
//! spawns a leaf worker, and parks; the leaf writes the shared state keys;
//! the barrier admits the final arrival and ONE notification wakes the
//! parent, which completes the run.
//!
//! This is the replacement topology for the removed trigger→spawn path, and
//! it pins the three claims that removal makes:
//!   * no trigger ever creates an agent — the only spawn is the parent's own
//!     direct `harness::spawn` call;
//!   * child outcomes flow ONLY through the medium — the parent's transcript
//!     gains exactly one `[notification]` (the barrier payload) and never a
//!     `[child-failure]` or injected child result;
//!   * the leaf runs on the sub-agent identity and needs no trigger
//!     knowledge — its generations are plain task → `state::set` writes.
//!
//! The leaf runs in its own session (untracked by the probe), so its turns
//! appear nowhere in the primary floor; its generations being consumed and
//! the parent actually WAKING are the evidence it ran and wrote.

use serde_json::{json, Value};

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const SPAWN: &str = "harness::spawn";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-005";
    const MESSAGE: &str = "Run the two-item pipeline.";

    let model = Model::scripted("fixture-model");

    // ONE barrier-gated once-wake over both item keys: skip at 1/2, allow at
    // 2/2 with the accumulated arrivals as the wake payload. The completion
    // condition is the PARENT's choice, expressed as data — the spec's
    // "barrier, or another explicit aggregate".
    let register_args = json!({
        "trigger_type": "state",
        "config": { "scope": "{{run_id}}" },
        "label": "pipeline-gate",
        "conditions": [{
            "function_id": "state::barrier",
            "config": { "id": "{{run_id}}-gate", "expect": ["items-1", "items-2"] }
        }]
    });

    // The leaf's whole world: one self-contained task naming the destination.
    // No trigger vocabulary, no parent, no siblings. Its policy is narrowed
    // to the single function the task needs.
    let spawn_args = json!({
        "task": "Write state scope {{run_id}} key items-1 value {\"item\":1,\"status\":\"done\"}, \
                 then key items-2 value {\"item\":2,\"status\":\"done\"}. Then stop.",
        "session_id": "{{run_id}}-leaf",
        "options": { "functions": { "allow": ["state::set"] } }
    });

    Scenario::new(
        ID,
        "direct-spawn-leaf-pipeline",
        "The parent registers a barrier wake, spawns a leaf directly, parks, and is woken \
         exactly once by the leaf's state writes.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-005")
            .allow_id(REGISTER)
            .allow_id(SPAWN)
            .allow_id("state::set"),
    )
    // The arm-and-spawn turn completes PARKED (the barrier wake is armed);
    // the woken turn is the single terminal one.
    .terminal_turn_statuses(["completed", "completed"])
    .parked_completions(1)
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
    // The leaf's opening step arrives BEFORE the parent's post-spawn step:
    // the child's turn job is enqueued during the spawn dispatch, ahead of
    // the parent's own re-enqueued next step (FIFO harness-turn queue).
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex("You are an iii sub-agent")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-w1",
                "state::set",
                json!({ "scope": "{{run_id}}", "key": "items-1",
                        "value": { "item": 1, "status": "done" } }),
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
            .respond(Response::text("wired and spawned", 10, 2)),
    )
    .generation(
        Generation::new(5)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex("You are an iii sub-agent")
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
                json!({ "scope": "{{run_id}}", "key": "items-2",
                        "value": { "item": 2, "status": "done" } }),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(6)
            .expect(
                Request::new()
                    .turn_request_step(2)
                    .system_prompt_regex("You are an iii sub-agent")
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
            .respond(Response::text("both items written", 10, 2)),
    )
    // The barrier-admitted wake: the parent's woken turn. Binding retirement
    // (once) races the turn start, so match the prompt loosely (INT-008 way).
    .generation(
        Generation::new(7)
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
            .respond(Response::text("pipeline complete", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["wired and spawned", "pipeline complete"])?;

        // Exactly one notification, carrying the barrier's aggregate — both
        // arrivals — because the condition's payload REPLACED the raw event.
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
            "the barrier must admit exactly ONE wake, got {}: {notifications:?}",
            notifications.len()
        );
        anyhow::ensure!(
            notifications[0].contains("items-1") && notifications[0].contains("items-2"),
            "the wake must carry the barrier's accumulated arrivals: {}",
            notifications[0]
        );

        // No second channel: nothing anywhere in the parent's transcript is a
        // child-failure injection or a child result outside the medium.
        for item in &run.transcript {
            let text = serde_json::to_string(item).unwrap_or_default();
            anyhow::ensure!(
                !text.contains("[child-failure]"),
                "a child outcome was injected outside the medium: {text}"
            );
        }

        // Two delivery records: the 1/2 arrival skipped by the barrier, and
        // the delivered 2/2 wake that retired the once binding.
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
            "expected a skip record and a delivered record, got {}",
            records.len()
        );
        let data = |r: &Value| r.get("custom").and_then(|c| c.get("data")).cloned();
        let skipped = records
            .iter()
            .filter_map(|r| data(r))
            .filter(|d| d.get("retired").and_then(Value::as_bool) == Some(false))
            .count();
        let retired = records
            .iter()
            .filter_map(|r| data(r))
            .filter(|d| d.get("retired").and_then(Value::as_bool) == Some(true))
            .count();
        anyhow::ensure!(
            skipped == 1 && retired == 1,
            "expected one barrier skip + one retiring delivery"
        );

        // The park resolved: nothing armed, nothing expected.
        anyhow::ensure!(
            run.status.get("expects_wake").and_then(Value::as_bool) == Some(false),
            "the session must not expect a wake after the barrier fired: {}",
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
    fn fixture_declares_the_park_and_the_leaf_generations() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_turn_statuses.len(), 2);
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.expected_traces(), 2);
        // Seven generations: three parent, three leaf, one woken parent. The
        // leaf's are consumed in its own session — the floor's
        // all-generations-consumed check is what proves the leaf ran.
        assert_eq!(fixture.script.generations.len(), 7);
        assert!(
            fixture.probe_actions.is_empty(),
            "the leaf writes the medium itself"
        );
    }
}
