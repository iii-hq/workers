//! Smoke tests for Azure OpenAI Responses wire shape + the templated
//! `api_url`.

use harness_types::{
    AgentMessage, AgentTool, ContentBlock, ExecutionMode, TextContent, UserMessage,
};
use provider_azure_openai::{
    to_responses_input, tools_to_responses, AzureOpenAIConfig, DEFAULT_API_VERSION, PROVIDER_NAME,
};

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_azure_openai::register_with_iii;
}

#[test]
fn public_constants_match_source() {
    assert_eq!(PROVIDER_NAME, "azure-openai");
    assert_eq!(DEFAULT_API_VERSION, "2025-01-01-preview");
}

#[test]
fn api_url_combines_resource_and_api_version() {
    let cfg = AzureOpenAIConfig {
        api_key: "k".into(),
        resource: "myresource".into(),
        deployment: "gpt-4o".into(),
        api_version: DEFAULT_API_VERSION.into(),
        max_output_tokens: 4096,
    };
    let url = cfg.api_url();
    assert_eq!(
        url,
        "https://myresource.openai.azure.com/openai/responses?api-version=2025-01-01-preview"
    );
}

#[test]
fn config_serde_round_trips() {
    let cfg = AzureOpenAIConfig {
        api_key: "k".into(),
        resource: "r".into(),
        deployment: "gpt-4o".into(),
        api_version: DEFAULT_API_VERSION.into(),
        max_output_tokens: 4096,
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: AzureOpenAIConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.deployment, "gpt-4o");
    assert_eq!(back.api_version, DEFAULT_API_VERSION);
}

#[test]
fn to_responses_input_emits_input_text_for_user() {
    let messages = vec![AgentMessage::User(UserMessage {
        content: vec![ContentBlock::Text(TextContent {
            text: "hello".into(),
        })],
        timestamp: 1,
    })];
    let out = to_responses_input(&messages, "system", false);
    assert_eq!(out[0]["role"], "system");
    assert_eq!(out[1]["role"], "user");
    assert_eq!(out[1]["content"][0]["type"], "input_text");
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
