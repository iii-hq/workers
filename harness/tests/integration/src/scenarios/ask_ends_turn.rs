//! INT-037 — a valid `harness::ask` ends the turn on the question.
//!
//! The model's only generation calls `harness::ask`. The turn loop answers the
//! call itself (the card data lands as the function result, `details.status:
//! "awaiting_answer"`) and completes the turn WITHOUT a second model call: the
//! user's answer arrives later as their next message. The script has exactly
//! one generation, so a second `router::chat` is an `unexpected_call` that
//! fails the turn.

use serde_json::json;

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const ASK: &str = "harness::ask";
const CALL_ID: &str = "call-ask";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-037";
    const MESSAGE: &str = "Ask me which approach to take.";

    let model = Model::scripted("fixture-model");
    let arguments = json!({ "questions": [ {
        "header": "Approach",
        "question": "When the agent asks, does the turn pause or end?",
        "options": [
            { "label": "Pause the turn", "description": "Like AskUserQuestion" },
            { "label": "End the turn", "description": "The answer becomes the next message" }
        ]
    } ] });

    Scenario::new(
        ID,
        "ask-ends-turn",
        "A valid harness::ask is answered by the turn loop and ends the turn without a second \
         model call.",
        ScenarioDriver::Direct,
        model,
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-036")
            .allow_id(ASK),
    )
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(CALL_ID, ASK, arguments, 8, 4)),
    )
    .verify(|run| {
        anyhow::ensure!(
            run.generations_consumed == 1,
            "expected exactly one router call, got {}",
            run.generations_consumed
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
        anyhow::ensure!(
            details["questions"][0]["header"] == json!("Approach"),
            "details.questions lost the card: {details}"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_one_generation_answered_by_a_single_ask() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.script.generations.len(), 1);
        let script = serde_json::to_string(&fixture.script.generations[0]).unwrap();
        assert!(script.contains(ASK), "{script}");
        assert!(script.contains(CALL_ID), "{script}");
    }
}
