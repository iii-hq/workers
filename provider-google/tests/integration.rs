//! Smoke tests for Gemini wire shape via the public helpers.

use harness_types::{
    AgentMessage, AgentTool, ContentBlock, ExecutionMode, StopReason, TextContent, UserMessage,
};
use provider_google::{
    map_finish_reason, to_wire_contents, tools_to_wire, GoogleConfig, DEFAULT_API_URL,
    PROVIDER_NAME,
};

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_google::register_with_iii;
}

#[test]
fn public_constants_match_source() {
    assert_eq!(PROVIDER_NAME, "google");
    assert_eq!(
        DEFAULT_API_URL,
        "https://generativelanguage.googleapis.com/v1beta/models"
    );
}

#[test]
fn config_serde_round_trips() {
    let cfg = GoogleConfig {
        api_key: "k".into(),
        model: "gemini-2.5-flash".into(),
        max_output_tokens: 4096,
        api_url: DEFAULT_API_URL.into(),
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: GoogleConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.model, "gemini-2.5-flash");
    assert_eq!(back.api_url, DEFAULT_API_URL);
}

#[test]
fn to_wire_contents_user_message_uses_user_role_with_text_part() {
    let messages = vec![AgentMessage::User(UserMessage {
        content: vec![ContentBlock::Text(TextContent {
            text: "hello".into(),
        })],
        timestamp: 1,
    })];
    let wire = to_wire_contents(&messages);
    assert_eq!(wire[0]["role"], "user");
    assert_eq!(wire[0]["parts"][0]["text"], "hello");
}

#[test]
fn tools_to_wire_wraps_in_function_declarations() {
    let tools = vec![AgentTool {
        name: "read".into(),
        description: "Read a file".into(),
        parameters: serde_json::json!({"type": "object"}),
        label: "Read".into(),
        execution_mode: ExecutionMode::Parallel,
        prepare_arguments_supported: false,
    }];
    let wire = tools_to_wire(&tools);
    assert_eq!(wire.len(), 1);
    assert_eq!(wire[0]["functionDeclarations"][0]["name"], "read");
}

#[test]
fn tools_to_wire_empty_yields_empty() {
    assert!(tools_to_wire(&[]).is_empty());
}

#[test]
fn map_finish_reason_known_values() {
    assert!(matches!(map_finish_reason("STOP"), StopReason::End));
    assert!(matches!(
        map_finish_reason("MAX_TOKENS"),
        StopReason::Length
    ));
    assert!(matches!(map_finish_reason("OTHER"), StopReason::End));
}
