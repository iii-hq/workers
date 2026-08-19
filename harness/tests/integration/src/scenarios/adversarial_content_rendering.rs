//! UI-SEC-001 — hostile-looking transcript and function data remain inert
//! while crossing the Harness durability boundary and rendering in Console.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send, Tool,
};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const ID: &str = "UI-SEC-001";
const INITIAL_MESSAGE: &str = "Render the adversarial fixture without executing it.";
const USER_PAYLOAD: &str = r#"USER_ATTACK_SENTINEL <img src="https://hostile.invalid/user.png" onerror="globalThis.__consoleUserXss=1"> [unsafe user link](javascript:globalThis.__consoleUserXss=2)"#;
const TOOL_ARGUMENT: &str = r#"TOOL_ATTACK_SENTINEL </code><img src="x" onerror="globalThis.__consoleToolXss=1"><script>globalThis.__consoleToolXss=2</script>"#;
const TOOL_RESULT: &str =
    r#"TOOL_RESULT_SENTINEL </pre><svg onload="globalThis.__consoleResultXss=1"></svg>"#;
const ASSISTANT_TEXT: &str = r#"ASSISTANT_ATTACK_SENTINEL

[unsafe assistant link](javascript:globalThis.__consoleAssistantXss=3)
![unsafe data image](data:image/svg+xml;base64,PHN2ZyBvbmxvYWQ9Imdsb2JhbFRoaXMuX19jb25zb2xlQXNzaXN0YW50WHNzPTQiPjwvc3ZnPg==)
[safe external link](https://example.com/adversarial-content)

<script>globalThis.__consoleAssistantXss = 1</script>
<img src="https://hostile.invalid/assistant.png" onerror="globalThis.__consoleAssistantXss=2">"#;
const USER_ACK: &str = "USER_ATTACK_ACK";

pub(super) fn scenario() -> ScenarioFixture {
    const CALL_ID: &str = "hostile-call";

    let model = Model::scripted("fixture-model");
    let echo = ControlledFunction::new(
        "{{run_id}}::adversarial_echo",
        "Return adversarial-looking text as inert test data.",
    )
    .request_schema(json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "value": { "type": "string" } },
        "required": ["value"]
    }))
    .returns_text(TOOL_RESULT);
    let arguments = json!({ "value": TOOL_ARGUMENT });

    Scenario::new(
        ID,
        "adversarial-content-rendering",
        "Hostile-looking user, model, and function data stays inert in the durable Console transcript.",
        ScenarioDriver::Playground,
        model.clone(),
    )
    .send(
        Send::message(INITIAL_MESSAGE)
            .idempotency_key("{{run_id}}:ui-sec-001")
            .allow_function(&echo),
    )
    .function(echo.clone())
    .terminal_turns(2)
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(INITIAL_MESSAGE)])
                    .tools_exact([echo.tool()]),
            )
            .respond(Response::function_call(
                CALL_ID,
                &echo,
                arguments.clone(),
                32,
                8,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([
                        Message::user(INITIAL_MESSAGE),
                        Message::function_call(
                            CALL_ID,
                            &echo,
                            arguments.clone(),
                            &model,
                            32,
                            8,
                        ),
                        Message::function_result(CALL_ID, &echo, TOOL_RESULT),
                    ])
                    .tools_exact([echo.tool()]),
            )
            .respond(Response::text(ASSISTANT_TEXT, 64, 24)),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex("agent_trigger")
                    .messages_exact([
                        Message::user(INITIAL_MESSAGE),
                        Message::function_call(
                            CALL_ID,
                            &echo,
                            arguments.clone(),
                            &model,
                            32,
                            8,
                        ),
                        Message::function_result(CALL_ID, &echo, TOOL_RESULT),
                        Message::assistant_text(ASSISTANT_TEXT, &model, 64, 24),
                        Message::user(USER_PAYLOAD),
                    ])
                    .tools_subset([Tool::named("agent_trigger")]),
            )
            .respond(Response::text(USER_ACK, 72, 4)),
    )
    .verify(|run| {
        run.expect_assistant_texts([ASSISTANT_TEXT, USER_ACK])?;
        run.expect_message_counts(2, 3, 1)?;
        run.expect_function_calls("adversarial_echo", 1)?;
        run.expect_call_payload("adversarial_echo", json!({ "value": TOOL_ARGUMENT }))?;
        let function_id = format!("{}::adversarial_echo", run.run_id);
        let results = run.function_results(&function_id);
        anyhow::ensure!(
            results.len() == 1
                && crate::evidence_data::message_text(results[0]) == TOOL_RESULT,
            "hostile function result was not retained exactly once"
        );
        anyhow::ensure!(
            run.generations_consumed == 3 && run.generations_total == 3,
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
    use crate::types::script::JsonMatcherV1;

    #[test]
    fn pins_hostile_values_as_plain_function_history() {
        let fixture = scenario();
        let JsonMatcherV1::Exact { expected, .. } = &fixture.script.generations[1].match_.messages
        else {
            panic!("function history must be exact")
        };
        assert_eq!(expected[0]["content"][0]["text"], INITIAL_MESSAGE);
        assert_eq!(
            expected[1]["content"][0]["arguments"]["value"],
            TOOL_ARGUMENT
        );
        assert_eq!(expected[2]["content"][0]["text"], TOOL_RESULT);
        assert_eq!(fixture.expected_terminal_turns, 2);
    }
}
