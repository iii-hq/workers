//! Smoke tests for the OpenAI Responses API wire shape + public surface.
//! Exercises `to_responses_input` and `tools_to_responses` directly so a
//! drift in `input_text` / `function_call` / `function_call_output` tags
//! is caught here.

use harness_types::{
    AgentMessage, AgentTool, ContentBlock, ExecutionMode, TextContent, ToolResultMessage,
    UserMessage,
};
use provider_openai_responses::{
    to_responses_input, tools_to_responses, OpenAIResponsesConfig, DEFAULT_API_URL, PROVIDER_NAME,
};

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_openai_responses::register_with_iii;
}

#[test]
fn public_constants_match_source() {
    assert_eq!(PROVIDER_NAME, "openai-responses");
    assert_eq!(DEFAULT_API_URL, "https://api.openai.com/v1/responses");
}

#[test]
fn config_serde_round_trips() {
    let cfg = OpenAIResponsesConfig {
        api_key: "sk-test".into(),
        model: "gpt-4o".into(),
        max_output_tokens: 4096,
        api_url: DEFAULT_API_URL.into(),
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: OpenAIResponsesConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.api_key, "sk-test");
    assert_eq!(back.model, "gpt-4o");
    assert_eq!(back.max_output_tokens, 4096);
}

#[test]
fn to_responses_input_emits_input_text_for_user() {
    let messages = vec![AgentMessage::User(UserMessage {
        content: vec![ContentBlock::Text(TextContent {
            text: "hello".into(),
        })],
        timestamp: 1,
    })];
    let out = to_responses_input(&messages, "be brief", false);
    // First entry is the system prompt with role=system.
    assert_eq!(out[0]["role"], "system");
    assert_eq!(out[0]["content"], "be brief");
    // Second entry is the user message with `input_text` typed content.
    assert_eq!(out[1]["role"], "user");
    assert_eq!(out[1]["content"][0]["type"], "input_text");
    assert_eq!(out[1]["content"][0]["text"], "hello");
}

#[test]
fn to_responses_input_uses_developer_role_for_reasoning_models() {
    let out = to_responses_input(&[], "system text", true);
    assert_eq!(out[0]["role"], "developer");
}

#[test]
fn to_responses_input_emits_function_call_output_for_tool_result() {
    let messages = vec![AgentMessage::ToolResult(ToolResultMessage {
        tool_call_id: "tc1".into(),
        tool_name: "read".into(),
        content: vec![ContentBlock::Text(TextContent { text: "ok".into() })],
        details: serde_json::json!({}),
        is_error: false,
        timestamp: 2,
    })];
    let out = to_responses_input(&messages, "", false);
    assert_eq!(out[0]["type"], "function_call_output");
    assert_eq!(out[0]["call_id"], "tc1");
    assert_eq!(out[0]["output"], "ok");
}

#[test]
fn tools_to_responses_emits_function_envelope() {
    let tools = vec![AgentTool {
        name: "read".into(),
        description: "Read a file".into(),
        parameters: serde_json::json!({"type": "object"}),
        label: "Read".into(),
        execution_mode: ExecutionMode::Parallel,
        prepare_arguments_supported: false,
    }];
    let out = tools_to_responses(&tools);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["name"], "read");
}
