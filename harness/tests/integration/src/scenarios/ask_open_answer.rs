//! INT-047 — an accepted ask remains durable while the session keeps moving,
//! then a later operator answer is delivered exactly as a new user message.

use serde_json::json;

use super::ScenarioDriver;
use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send,
};
use crate::fixtures::ScenarioFixture;

const ASK: &str = "harness::ask";
const ANSWER: &str = "harness::ask::answer";
const LIST: &str = "harness::ask::list";
const ON_ASK: &str = "harness::on-ask";
const ON_SETTLED: &str = "harness::on-ask-settled";
const ON_COMPLETED: &str = "harness::on-turn-completed";
const QUESTION_ID: &str = "call-ask";
const INITIAL: &str = "Try the unsafe expiry default, then get ready for the real ask.";
const ASK_NOW: &str = "Ask which approach to use, then keep working if I steer you.";
const STEER: &str = "Keep going on the independent checks while the question stays open.";
const ANSWER_TEXT: &str = "Approach: Continue";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-047";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new("{{run_id}}::record", "Record a harness ask event.")
        .request_schema(json!({ "type": "object" }))
        .returns_text("recorded");
    let ask = json!({
        "questions": [{
            "header": "Approach",
            "question": "Should I continue the independent checks?",
            "options": [
                { "label": "Continue", "recommended": true },
                { "label": "Pause" }
            ]
        }],
        "about": {
            "ticket": "KAN-47",
            "blocks": ["KAN-48"],
            "class": "scope",
            "why": "The independent checks do not depend on this choice."
        }
    });
    let invalid_expiry = json!({
        "questions": [{
            "header": "Security",
            "question": "Publish with the unresolved security finding?",
            "options": [{ "label": "Publish" }, { "label": "Wait" }]
        }],
        "about": { "class": "security" },
        "expires_in_min": 1,
        "on_expiry": { "answer": "Publish" }
    });

    Scenario::new(
        ID,
        "ask-open-answer",
        "An ask is listed and observable while steering lets the session continue; a later \
         operator answer becomes one message, and an agent still cannot answer asks.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(INITIAL)
            .idempotency_key("{{run_id}}:integration-047")
            .allow_id(ASK)
            // Deliberately allowed: the harness's non-overridable boundary
            // must still refuse an agent call to answer.
            .allow_id(ANSWER),
    )
    .function(record.clone())
    .terminal_turns(3)
    // on-ask, open turn-completed, on-ask-settled, closed turn-completed.
    .await_target_calls(4)
    // Install all event observers before the turn that asks, then start it.
    .probe_observe_after(1, ON_ASK, json!({}), record.id())
    .probe_observe_after(1, ON_SETTLED, json!({}), record.id())
    .probe_observe_after(1, ON_COMPLETED, json!({}), record.id())
    .probe_after(
        1,
        "harness::send",
        json!({ "session_id": "{{session_id}}", "message": ASK_NOW }),
    )
    // Reading at the second completion proves the question survived the
    // steering turn as an open durable entry.
    .probe_after_matching(
        2,
        LIST,
        json!({ "session_id": "{{session_id}}" }),
        json!({
            "entries": [{
                "question_id": QUESTION_ID,
                "state": "open",
                "about": { "ticket": "KAN-47", "class": "scope" }
            }]
        }),
    )
    .probe_after(
        2,
        ANSWER,
        json!({
            "session_id": "{{session_id}}",
            "question_id": QUESTION_ID,
            "answers": [{ "header": "Approach", "choice": "Continue" }],
            "by": { "channel": "tower", "user": "integration" }
        }),
    )
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(INITIAL)])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-invalid-expiry",
                ASK,
                invalid_expiry,
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
                        Message::user(INITIAL),
                        json!({ "role": "function_result",
                                "function_call_id": "call-invalid-expiry", "is_error": true }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("ready", 10, 1)),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([Message::user(ASK_NOW)])
                    .tools_subset([]),
            )
            // The steering message lands before the ask result settles. The
            // accepted ask must remain open while this turn advances.
            .parked_message(STEER)
            .respond(Response::function_call_raw(QUESTION_ID, ASK, ask, 16, 5)),
    )
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        Message::user(ASK_NOW),
                        json!({ "role": "function_result", "function_call_id": QUESTION_ID,
                                "is_error": false }),
                        Message::user(STEER),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("independent checks complete", 20, 3)),
    )
    .generation(
        Generation::new(5)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([Message::user(ANSWER_TEXT)])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-agent-answer",
                ANSWER,
                json!({
                    "session_id": "{{session_id}}",
                    "question_id": QUESTION_ID,
                    "answers": [{ "header": "Approach", "choice": "Pause" }],
                    "by": { "channel": "chat" }
                }),
                30,
                5,
            )),
    )
    .generation(
        Generation::new(6)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        Message::user(ANSWER_TEXT),
                        json!({ "role": "function_result",
                                "function_call_id": "call-agent-answer", "is_error": true }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("answer received", 36, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["ready", "independent checks complete", "answer received"])?;
        let ask_results = run.function_results(ASK);
        let accepted = ask_results
            .iter()
            .filter(|result| result["is_error"] == false)
            .count();
        let refused = ask_results
            .iter()
            .filter(|result| result["is_error"] == true)
            .count();
        anyhow::ensure!(
            accepted == 1
                && refused == 1
                && ask_results
                    .iter()
                    .any(|result| result["details"]["question_id"] == QUESTION_ID),
            "accepted ask result missing or duplicated: {ask_results:?}"
        );
        let denied = run.function_results(ANSWER);
        anyhow::ensure!(
            denied.len() == 1 && denied[0]["is_error"] == true,
            "agent answer must be denied under an explicit allow policy: {denied:?}"
        );
        anyhow::ensure!(
            run.status["open_asks"] == 0,
            "settled final status must report no open asks: {}",
            run.status
        );

        let recorded = run
            .target_calls
            .iter()
            .map(|call| serde_json::to_string(call).unwrap_or_default())
            .collect::<Vec<_>>();
        anyhow::ensure!(
            recorded.iter().any(|call| {
                call.contains(r#""question_id":"call-ask""#) && call.contains(r#""state":"open""#)
            }),
            "harness::on-ask did not carry the open entry: {recorded:?}"
        );
        anyhow::ensure!(
            recorded.iter().any(|call| {
                call.contains(r#""question_id":"call-ask""#)
                    && call.contains(r#""state":"answered""#)
            }),
            "harness::on-ask-settled did not carry the answered entry: {recorded:?}"
        );
        anyhow::ensure!(
            recorded
                .iter()
                .any(|call| call.contains(r#""open_asks":1"#)),
            "the open turn-completed event did not report open_asks=1: {recorded:?}"
        );

        let answer_messages = run
            .transcript
            .iter()
            .filter_map(|item| item.get("message"))
            .filter(|message| {
                message["role"] == "user"
                    && serde_json::to_string(message).is_ok_and(|wire| wire.contains(ANSWER_TEXT))
            })
            .count();
        anyhow::ensure!(
            answer_messages == 1,
            "operator answer was delivered {answer_messages} times"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_observes_lists_answers_and_checks_the_agent_boundary() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.slug, "ask-open-answer");
        assert_eq!(fixture.expected_terminal_turns, 3);
        assert_eq!(fixture.await_target_calls, Some(4));
        assert_eq!(fixture.script.generations.len(), 6);
        assert_eq!(fixture.probe_actions.len(), 6);
        assert!(fixture.probe_actions[4].expected_output.is_some());
    }
}
