//! E2E-001 — streamed text reaches durable completion.

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
    const ID: &str = "E2E-001";
    const MESSAGE: &str = "Return the fixture phrase.";
    const TEXT: &str = "fixture complete";

    let model = model();
    let usage = usage(8, 2);
    let allowed_functions = Vec::new();
    let messages = vec![user_message(MESSAGE)];
    let generation = ScriptedGenerationV1 {
        ordinal: 1,
        match_: request_match(1, &model, &messages, &json!([]), RequestProfile::Direct),
        frames: streamed_text_frames(TEXT, &["fixture ", "complete"], &usage, &model),
        response: response(StopReason::End, usage, &model),
    };

    ScenarioFixture {
        slug: "streamed-text".to_string(),
        driver: ScenarioDriver::Direct,
        scenario: CompiledScenarioV1 {
            schema_version: SchemaVersion1::V1,
            id: ID.to_string(),
            description:
                "Streamed text reaches durable completion through the real queue and turn loop."
                    .to_string(),
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
            let texts = run.assistant_texts();
            ensure!(texts == [TEXT], "assistant texts {texts:?} != [\"{TEXT}\"]");
            let counts = run.message_counts();
            ensure!(
                counts == (1, 1, 0),
                "message counts (user, assistant, function_result) {counts:?} != (1, 1, 0)"
            );
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
