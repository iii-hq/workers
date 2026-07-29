//! UI-001 — Playwright sends a Console turn and validates the rendered result.

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send, Tool};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "UI-001";
    const MESSAGE: &str = "Return the console fixture phrase.";
    const TEXT: &str = "console fixture complete";

    Scenario::new(
        ID,
        "console-streamed-text",
        "A Console-sent streamed turn reaches durable completion.",
        ScenarioDriver::Playground,
        Model::scripted("fixture-model"),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:ui-001")
            .without_functions(),
    )
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_regex("agent_trigger")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_subset([Tool::named("agent_trigger")]),
            )
            .respond(Response::streamed_text(
                TEXT,
                ["console fixture ", "complete"],
                9,
                3,
            )),
    )
    .verify(|run| run.expect_no_duplicate_messages())
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::script::JsonMatcherV1;

    #[test]
    fn accepts_console_prompt_and_tool_shape() {
        let fixture = scenario();
        assert!(matches!(
            fixture.script.generations[0].match_.system_prompt,
            JsonMatcherV1::Regex { .. }
        ));
        assert!(matches!(
            fixture.script.generations[0].match_.tools,
            JsonMatcherV1::Subset { .. }
        ));
    }
}
