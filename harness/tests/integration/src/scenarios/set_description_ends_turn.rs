//! INT-045 — `harness::set-description` beside the reply ends the turn.
//!
//! chat-projects §1.2.3: the model's only generation answers in text and, in
//! the same message, calls `harness::set-description`. The turn loop answers
//! the call itself for the turn's own session (one `session::patch-meta`),
//! and — its step's only call being the description, beside reply text —
//! completes the turn WITHOUT another model request. The script has exactly
//! one generation, so a second `router::chat` is an `unexpected_call` that
//! fails the turn. `session::on-meta-updated` carries the new description.

use serde_json::json;

use super::ScenarioDriver;
use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use crate::fixtures::ScenarioFixture;

const SET_DESCRIPTION: &str = "harness::set-description";
const CALL_ID: &str = "call-describe";
const DESCRIPTION: &str = "Fixing the login form";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-045";
    const MESSAGE: &str = "The login form rejects valid emails; fix it.";

    let model = Model::scripted("fixture-model");
    Scenario::new(
        ID,
        "set-description-ends-turn",
        "harness::set-description beside reply text is answered by the turn loop, writes the \
         description through session::patch-meta, and ends the turn without another model call.",
        ScenarioDriver::Direct,
        model,
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-045")
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
                "The email check now accepts plus addresses.",
                CALL_ID,
                SET_DESCRIPTION,
                json!({ "description": DESCRIPTION, "title": "Login form fix" }),
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
            results.len() == 1,
            "expected one harness::set-description result, got {}",
            results.len()
        );
        let result = results[0];
        anyhow::ensure!(
            result["is_error"] == json!(false) && result["details"]["ok"] == json!(true),
            "the description was not written: {result}"
        );
        let update = run
            .meta_updates
            .iter()
            .find(|event| event["description"] == json!(DESCRIPTION))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no session::on-meta-updated carried the description: {:?}",
                    run.meta_updates
                )
            })?;
        anyhow::ensure!(
            update["metadata"]["description_by"] == json!("agent")
                && update["metadata"]["description_at_ms"].is_i64()
                && update["metadata"]["title_by"] == json!("agent")
                && update["title"] == json!("Login form fix"),
            "the metadata event lacks the agent's marks: {update}"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_one_generation_with_text_and_a_description() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.script.generations.len(), 1);
        let script = serde_json::to_string(&fixture.script.generations[0]).unwrap();
        assert!(script.contains(SET_DESCRIPTION), "{script}");
        assert!(script.contains(DESCRIPTION), "{script}");
        assert!(script.contains("plus addresses"), "{script}");
    }
}
