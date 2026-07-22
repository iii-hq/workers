//! E2E-001 — streamed text reaches durable completion.

use super::dsl::{Generation, Message, Model, Recorder, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "E2E-001";
    const MESSAGE: &str = "Return the fixture phrase.";
    const TEXT: &str = "fixture complete";

    Scenario::new(
        ID,
        "streamed-text",
        "Streamed text reaches durable completion through the real queue and turn loop.",
        ScenarioDriver::Direct,
        Model::scripted("fixture-model"),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:e2e-001")
            .without_functions(),
    )
    .recorder(Recorder::lifecycle_only())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .without_tools(),
            )
            .respond(Response::streamed_text(
                TEXT,
                ["fixture ", "complete"],
                8,
                2,
            )),
    )
    .verify(|run| {
        run.expect_assistant_texts([TEXT])?;
        run.expect_message_counts(1, 1, 0)?;
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::frames::StopReason;

    #[test]
    fn stream_has_one_terminal_frame_and_matching_response() {
        let fixture = scenario();
        let generation = &fixture.script.generations[0];
        assert_eq!(
            generation
                .frames
                .iter()
                .filter(|frame| frame.is_terminal())
                .count(),
            1
        );
        assert!(generation.frames.last().unwrap().is_terminal());
        assert_eq!(generation.response.stop_reason, Some(StopReason::End));
    }
}
