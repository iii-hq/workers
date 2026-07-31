//! UI-003 — the production Console keeps its composer editable while a turn is
//! streaming and queues a second message through the public harness surface.

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send, Tool};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

pub(super) const GATE: &str = "console-queued-message-streaming-in-flight";
pub(super) const QUEUED_MESSAGE: &str = "Queue this while the first response is streaming.";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "UI-003";
    const INITIAL_MESSAGE: &str = "Start the Console queueing fixture.";
    const FIRST_TEXT: &str = "first Console response complete";
    const FINAL_TEXT: &str = "queued Console message complete";

    let model = Model::scripted("fixture-model");

    Scenario::new(
        ID,
        "console-queued-message-streaming",
        "The Console composer stays editable during streaming and queues a second message in order.",
        ScenarioDriver::Playground,
        model.clone(),
    )
    .send(
        Send::message(INITIAL_MESSAGE)
            .idempotency_key("{{run_id}}:ui-003")
            .without_functions(),
    )
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_regex("agent_trigger")
                    .messages_exact([Message::user(INITIAL_MESSAGE)])
                    .tools_subset([Tool::named("agent_trigger")]),
            )
            .gate(GATE)
            .respond(Response::streamed_text(
                FIRST_TEXT,
                ["first Console ", "response complete"],
                12,
                3,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex("agent_trigger")
                    .messages_exact([
                        Message::user(INITIAL_MESSAGE),
                        Message::assistant_text(FIRST_TEXT, &model, 12, 3),
                        Message::user(QUEUED_MESSAGE),
                    ])
                    .tools_subset([Tool::named("agent_trigger")]),
            )
            .respond(Response::streamed_text(
                FINAL_TEXT,
                ["queued Console ", "message complete"],
                10,
                3,
            )),
    )
    .verify(|run| {
        run.expect_assistant_texts([FIRST_TEXT, FINAL_TEXT])?;
        run.expect_message_counts(2, 2, 0)?;
        run.expect_no_duplicate_messages()
    })
    .scenario_timeout_ms(60_000)
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_generation_is_gated_and_second_consumes_the_queued_message() {
        let fixture = scenario();
        assert_eq!(fixture.script.generations.len(), 2);
        assert_eq!(
            fixture.script.generations[0]
                .gate
                .as_ref()
                .map(|gate| gate.name.as_str()),
            Some(GATE)
        );
        assert!(serde_json::to_string(&fixture.script.generations[1])
            .unwrap()
            .contains(QUEUED_MESSAGE));
        assert!(
            serde_json::to_string(&fixture.script.generations[1].match_.request_id)
                .unwrap()
                .contains(":1$")
        );
    }
}
