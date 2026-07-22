//! UI-001 — Playwright sends a Console turn and validates the rendered result.

use anyhow::ensure;
use serde_json::json;

use super::support::{
    model, request_match, response, send, streamed_text_frames, synthetic_recorder, system_prompt,
    usage, user_message, RequestProfile,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;
use crate::types::frames::StopReason;
use crate::types::scenario::{CompiledScenarioV1, DeadlinesV1};
use crate::types::script::{RouterScriptV1, SchemaVersion1, ScriptedGenerationV1};

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "UI-001";
    const MESSAGE: &str = "Return the console fixture phrase.";
    const TEXT: &str = "console fixture complete";

    let model = model();
    let usage = usage(9, 3);
    let allowed_functions = Vec::new();
    let messages = vec![user_message(MESSAGE)];
    let generation = ScriptedGenerationV1 {
        ordinal: 1,
        match_: request_match(1, &model, &messages, &json!([]), RequestProfile::Console),
        frames: streamed_text_frames(TEXT, &["console fixture ", "complete"], &usage, &model),
        response: response(StopReason::End, usage, &model),
    };

    ScenarioFixture {
        slug: "console-streamed-text".to_string(),
        driver: ScenarioDriver::Playground,
        scenario: CompiledScenarioV1 {
            schema_version: SchemaVersion1::V1,
            id: ID.to_string(),
            description: "A Console-sent streamed turn reaches durable completion.".to_string(),
            send: send(ID, MESSAGE, &model, &allowed_functions),
            recorder: synthetic_recorder(),
            deadlines: DeadlinesV1::default(),
        },
        script: RouterScriptV1 {
            schema_version: SchemaVersion1::V1,
            scenario_id: ID.to_string(),
            model,
            generations: vec![generation],
        },
        system_prompt_template: system_prompt(&allowed_functions),
        verify: |run| {
            ensure!(
                !run.has_duplicate_messages(),
                "transcript contains duplicate entry ids"
            );
            Ok(())
        },
    }
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
