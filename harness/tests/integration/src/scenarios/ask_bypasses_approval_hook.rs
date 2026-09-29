//! INT-038 — `harness::ask` bypasses the pre_trigger hooks.
//!
//! `harness::ask` is a harness control like `submit_result`: an approval hook
//! that holds it would park the turn, and the deferred release path cannot
//! show the card. The fixture proves the ask never reaches the hook chain:
//!
//!   1. turn 1 answers in text, so the probe has a completion boundary;
//!   2. the probe binds a `harness::hook::pre-trigger` hook that HOLDS every
//!      call (`functions: ["*"]`, the controlled function answering
//!      `{ "decision": "hold" }`), then sends a second message;
//!   3. turn 2's only generation calls `harness::ask`. The loop answers it
//!      itself: `awaiting_answer`, nothing pending, turn completed, no third
//!      model call, and the hook is never consulted (zero controlled calls).
//!
//! Before the bypass the hook held the ask: turn 2 parked with a pending call
//! and never reached a terminal status.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const ASK: &str = "harness::ask";
const PRE_TRIGGER: &str = "harness::hook::pre-trigger";
const CALL_ID: &str = "call-ask";
const FIRST_MESSAGE: &str = "Say ready.";
const FIRST_TEXT: &str = "ready";
const SECOND_MESSAGE: &str = "Ask me which approach to take.";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-038";

    let model = Model::scripted("fixture-model");
    let gate = ControlledFunction::new(
        "{{run_id}}::gate",
        "Approval gate that holds every call it is consulted on.",
    )
    .request_schema(json!({ "type": "object" }))
    .returns_json(json!({ "decision": "hold" }));
    let arguments = json!({ "questions": [ {
        "header": "Approach",
        "question": "When the agent asks, does the turn pause or end?",
        "options": [
            { "label": "Pause the turn" },
            { "label": "End the turn" }
        ]
    } ] });

    Scenario::new(
        ID,
        "ask-bypasses-approval-hook",
        "A pre_trigger hook that holds every call does not hold harness::ask: the turn loop \
         answers it before the hook chain and the turn ends on the question.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(FIRST_MESSAGE)
            .idempotency_key("{{run_id}}:integration-038")
            .allow_id(ASK),
    )
    .terminal_turns(2)
    // Only the two sends make turns; the hook registration leaves no trace.
    .expect_traces(2)
    .probe_after(
        1,
        "engine::register_trigger",
        json!({
            "trigger_type": PRE_TRIGGER,
            "function_id": "{{run_id}}::gate",
            "config": { "functions": ["*"] }
        }),
    )
    // No options: the follow-up inherits the first turn's model, prompt and
    // policy (`harness::ask` allowed).
    .probe_after(
        1,
        "harness::send",
        json!({
            "session_id": "{{session_id}}",
            "message": SECOND_MESSAGE,
            "idempotency_key": "{{run_id}}:integration-038-b"
        }),
    )
    .function(gate)
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(FIRST_MESSAGE)])
                    .tools_subset([]),
            )
            .respond(Response::text(FIRST_TEXT, 8, 2)),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(ASK)
                    .messages_exact([
                        Message::user(FIRST_MESSAGE),
                        Message::assistant_text(FIRST_TEXT, &model, 8, 2),
                        Message::user(SECOND_MESSAGE),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(CALL_ID, ASK, arguments, 8, 4)),
    )
    .verify(|run| {
        anyhow::ensure!(
            run.generations_consumed == 2 && run.generations_total == 2,
            "{} of {} scripted generations consumed",
            run.generations_consumed,
            run.generations_total
        );
        anyhow::ensure!(
            run.target_calls.is_empty(),
            "the holding pre_trigger hook was consulted {} time(s): {:?}",
            run.target_calls.len(),
            run.target_calls
        );
        let results = run.function_results(ASK);
        anyhow::ensure!(
            results.len() == 1,
            "expected one harness::ask result, got {}",
            results.len()
        );
        let result = results[0];
        anyhow::ensure!(
            result["is_error"] == json!(false),
            "harness::ask result is an error: {result}"
        );
        let details = &result["details"];
        anyhow::ensure!(
            details["status"] == json!("awaiting_answer"),
            "details.status != awaiting_answer: {details}"
        );
        anyhow::ensure!(
            details["question_id"] == json!(CALL_ID),
            "details.question_id != {CALL_ID}: {details}"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_a_holding_hook_before_the_ask_turn() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.expected_traces(), 2);
        assert_eq!(fixture.script.generations.len(), 2);

        // The hook is bound after turn 1 and before the ask turn's send.
        let actions: Vec<_> = fixture
            .probe_actions
            .iter()
            .map(|action| (action.after_turns, action.function_id.as_str()))
            .collect();
        assert_eq!(
            actions,
            [(1, "engine::register_trigger"), (1, "harness::send")]
        );
        let hook = &fixture.probe_actions[0].payload;
        assert_eq!(hook["trigger_type"], PRE_TRIGGER);
        assert_eq!(hook["config"]["functions"], json!(["*"]));

        let ask_turn = serde_json::to_string(&fixture.script.generations[1]).unwrap();
        assert!(ask_turn.contains(ASK), "{ask_turn}");
        assert!(ask_turn.contains(CALL_ID), "{ask_turn}");
    }
}
