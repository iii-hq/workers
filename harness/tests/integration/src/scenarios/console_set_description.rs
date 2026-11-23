//! UI-006 — the Console shows a description the agent wrote in its turn.
//!
//! chat-projects §1.4 / §1.8 (4), driven by Playwright
//! (`ade/web/e2e/set-description.spec.ts`): the model's only generation
//! answers in text and, in the same message, calls `harness::set-description`
//! (no `title`, so the session keeps the name the test opens it by). The turn
//! loop writes the description through `session::patch-meta` and ends the
//! turn without another model request (one scripted generation: a second
//! `router::chat` would fail the turn). The Console then shows the
//! description in the chat header and the sidebar row, and no call card.

use serde_json::json;

use super::ScenarioDriver;
use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use crate::fixtures::ScenarioFixture;

const SET_DESCRIPTION: &str = "harness::set-description";
const CALL_ID: &str = "call-console-describe";
const DESCRIPTION: &str = "Retries with backoff, then a PR";
const REPLY: &str = "console description fixture complete";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "UI-006";
    const MESSAGE: &str = "Add retries with backoff to the webhook sender.";

    Scenario::new(
        ID,
        "console-set-description",
        "A Console turn whose reply comes with harness::set-description: the description reaches \
         the header and the sidebar, the transcript shows no call, and the turn makes one model call.",
        ScenarioDriver::Playground,
        Model::scripted("fixture-model"),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:ui-006")
            .allow_id(SET_DESCRIPTION),
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
            .respond(Response::text_and_function_call_raw(
                REPLY,
                CALL_ID,
                SET_DESCRIPTION,
                json!({ "description": DESCRIPTION }),
                8,
                6,
            )),
    )
    .verify(|run| {
        anyhow::ensure!(
            run.generations_consumed == 1,
            "expected exactly one router call, got {}",
            run.generations_consumed
        );
        let results = run.function_results(SET_DESCRIPTION);
        anyhow::ensure!(
            results.len() == 1
                && results[0]["is_error"] == json!(false)
                && results[0]["details"]["ok"] == json!(true),
            "the description was not written: {results:?}"
        );
        anyhow::ensure!(
            run.meta_updates
                .iter()
                .any(|event| event["description"] == json!(DESCRIPTION)
                    && event["metadata"]["description_by"] == json!("agent")),
            "no session::on-meta-updated carried the agent's description: {:?}",
            run.meta_updates
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_one_generation_with_reply_text_and_a_description() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.driver, ScenarioDriver::Playground);
        assert_eq!(fixture.script.generations.len(), 1);
        let script = serde_json::to_string(&fixture.script.generations[0]).unwrap();
        assert!(script.contains(SET_DESCRIPTION), "{script}");
        assert!(script.contains(DESCRIPTION), "{script}");
        assert!(script.contains(REPLY), "{script}");
        // No title: the Console opens the session by the name it was sent with.
        assert!(!script.contains("\"title\""), "{script}");
    }
}
