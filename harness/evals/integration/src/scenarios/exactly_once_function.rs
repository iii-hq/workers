//! E2E-002 — the recorder runs exactly once and its result closes the turn.

use anyhow::ensure;
use serde_json::json;

use super::support::{
    assistant_message, model, recorder, recorder_target, request_match, response, send,
    system_prompt, usage, user_message, RequestProfile, MODEL_ID, PROVIDER_ID,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;
use crate::types::frames::{AssistantMessageEvent, ContentBlock, StopReason};
use crate::types::scenario::{CompiledScenarioV1, DeadlinesV1};
use crate::types::script::{RouterScriptV1, SchemaVersion1, ScriptedGenerationV1};

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "E2E-002";
    const MESSAGE: &str = "Call the recorder once.";
    const FUNCTION_ID: &str = "{{run_id}}::record";

    let model = model();
    let target = recorder_target(FUNCTION_ID);
    let allowed_functions = vec![FUNCTION_ID.to_string()];
    let tools = json!([{
        "name": FUNCTION_ID,
        "description": target.description,
        "parameters": target.request_schema,
        "execution_mode": "sequential"
    }]);
    let arguments = json!({ "value": "expected" });
    let call_usage = usage(8, 4);
    let final_usage = usage(18, 2);

    let first_messages = vec![user_message(MESSAGE)];
    let function_call = assistant_message(
        vec![ContentBlock::FunctionCall {
            id: "call-1".to_string(),
            function_id: FUNCTION_ID.to_string(),
            arguments: arguments.clone(),
        }],
        StopReason::FunctionCall,
        Some(call_usage.clone()),
        &model,
        1,
    );
    let mut second_messages = first_messages.clone();
    second_messages.extend([
        json!({
            "role": "assistant",
            "content": [{
                "type": "function_call",
                "id": "call-1",
                "function_id": FUNCTION_ID,
                "arguments": arguments
            }],
            "stop_reason": "end",
            "model": MODEL_ID,
            "provider": PROVIDER_ID
        }),
        json!({
            "role": "function_result",
            "function_call_id": "call-1",
            "function_id": FUNCTION_ID,
            "content": [{ "type": "text", "text": "recorded" }],
            "details": target.response,
            "is_error": false
        }),
    ]);
    let final_message = assistant_message(
        vec![ContentBlock::Text {
            text: "recorded once".to_string(),
        }],
        StopReason::End,
        Some(final_usage.clone()),
        &model,
        2,
    );
    let generations = vec![
        ScriptedGenerationV1 {
            ordinal: 1,
            match_: request_match(1, &model, &first_messages, &tools, RequestProfile::Direct),
            frames: vec![AssistantMessageEvent::Done {
                message: function_call,
            }],
            response: response(StopReason::FunctionCall, call_usage, &model),
        },
        ScriptedGenerationV1 {
            ordinal: 2,
            match_: request_match(2, &model, &second_messages, &tools, RequestProfile::Direct),
            frames: vec![AssistantMessageEvent::Done {
                message: final_message,
            }],
            response: response(StopReason::End, final_usage, &model),
        },
    ];

    ScenarioFixture {
        slug: "exactly-once-function".to_string(),
        driver: ScenarioDriver::Direct,
        scenario: CompiledScenarioV1 {
            schema_version: SchemaVersion1::V1,
            id: ID.to_string(),
            description: "The recorder runs exactly once.".to_string(),
            send: send(ID, MESSAGE, &model, &allowed_functions),
            recorder: recorder(target),
            deadlines: DeadlinesV1::default(),
        },
        script: RouterScriptV1 {
            schema_version: SchemaVersion1::V1,
            scenario_id: ID.to_string(),
            model,
            generations,
        },
        system_prompt_template: system_prompt(&allowed_functions),
        verify: |run| {
            let texts = run.assistant_texts();
            ensure!(
                texts == ["recorded once"],
                "assistant texts {texts:?} != [\"recorded once\"]"
            );
            let calls = run.calls("record");
            ensure!(
                calls.len() == 1,
                "record ran {} times, not exactly once",
                calls.len()
            );
            let payload = &calls[0].payload;
            ensure!(
                payload == &json!({ "value": "expected" }),
                "record payload {payload} != {{\"value\":\"expected\"}}"
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
    use crate::types::script::JsonMatcherV1;

    #[test]
    fn pins_function_call_and_result_history() {
        let fixture = scenario();
        let JsonMatcherV1::Exact { expected, .. } = &fixture.script.generations[1].match_.messages
        else {
            panic!("function history must be exact")
        };
        assert_eq!(expected.as_array().unwrap().len(), 3);
        assert_eq!(expected[1]["content"][0]["id"], "call-1");
        assert_eq!(expected[2]["role"], "function_result");
    }
}
