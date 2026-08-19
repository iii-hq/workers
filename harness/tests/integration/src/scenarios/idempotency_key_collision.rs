//! INT-022 — distinct idempotency keys must never collapse onto one durable
//! user entry and silently remove the later message.

use serde_json::json;

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const ID: &str = "INT-022";
const FIRST_MESSAGE: &str = "Record the first collision-control message.";
const SECOND_MESSAGE: &str = "Record the second collision-control message.";
const FIRST_TEXT: &str = "first collision-control turn complete";
const SECOND_TEXT: &str = "second collision-control turn complete";

pub(super) fn scenario() -> ScenarioFixture {
    let model = Model::scripted("fixture-model");

    Scenario::new(
        ID,
        "idempotency-key-collision",
        "Distinct idempotency keys that contain punctuation retain distinct durable messages.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(FIRST_MESSAGE)
            .idempotency_key("{{run_id}}:idem/a")
            .without_functions(),
    )
    .terminal_turns(2)
    .probe_after(
        1,
        "harness::send",
        json!({
            "session_id": "{{session_id}}",
            "message": SECOND_MESSAGE,
            "idempotency_key": "{{run_id}}:idem a"
        }),
    )
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(FIRST_MESSAGE)])
                    .without_tools(),
            )
            .respond(Response::text(FIRST_TEXT, 12, 4)),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([
                        Message::user(FIRST_MESSAGE),
                        Message::assistant_text(FIRST_TEXT, &model, 12, 4),
                        Message::user(SECOND_MESSAGE),
                    ])
                    .without_tools(),
            )
            .respond(Response::text(SECOND_TEXT, 18, 5)),
    )
    .verify(|run| {
        run.expect_assistant_texts([FIRST_TEXT, SECOND_TEXT])?;
        run.expect_message_counts(2, 2, 0)?;
        anyhow::ensure!(
            run.generations_consumed == 2 && run.generations_total == 2,
            "{} of {} scripted generations consumed",
            run.generations_consumed,
            run.generations_total
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drives_the_second_distinct_key_after_the_first_turn() {
        let fixture = scenario();
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.probe_actions.len(), 1);
        assert_eq!(fixture.probe_actions[0].after_turns, 1);
        assert_eq!(fixture.probe_actions[0].function_id, "harness::send");
        assert_ne!(
            Some(fixture.scenario.send.idempotency_key.as_str()),
            fixture.probe_actions[0]
                .payload
                .get("idempotency_key")
                .and_then(serde_json::Value::as_str)
        );
    }
}
