//! Smoke tests for Vertex AI wire shape + the templated `api_url`.

use harness_types::{
    AgentMessage, AgentTool, ContentBlock, ExecutionMode, StopReason, TextContent, UserMessage,
};
use provider_google_vertex::{
    map_finish_reason, to_wire_contents, tools_to_wire, VertexConfig, DEFAULT_REGION, PROVIDER_NAME,
};

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_google_vertex::register_with_iii;
}

#[test]
fn public_constants_match_source() {
    assert_eq!(PROVIDER_NAME, "google-vertex");
    assert_eq!(DEFAULT_REGION, "us-central1");
}

#[test]
fn api_url_is_constructed_from_region_project_model() {
    let cfg = VertexConfig {
        access_token: "tok".into(),
        project: "my-proj".into(),
        region: "us-central1".into(),
        model: "gemini-2.5-flash".into(),
        max_output_tokens: 4096,
    };
    let url = cfg.api_url();
    assert!(url.contains("us-central1-aiplatform.googleapis.com"));
    assert!(url.contains("/projects/my-proj/"));
    assert!(url.contains("/models/gemini-2.5-flash:streamGenerateContent"));
    assert!(url.contains("alt=sse"));
}

#[test]
fn config_serde_round_trips() {
    let cfg = VertexConfig {
        access_token: "tok".into(),
        project: "my-proj".into(),
        region: DEFAULT_REGION.into(),
        model: "gemini-2.5-flash".into(),
        max_output_tokens: 4096,
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: VertexConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.project, "my-proj");
    assert_eq!(back.region, DEFAULT_REGION);
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
    assert_eq!(wire[0]["functionDeclarations"][0]["name"], "read");
}

#[test]
fn map_finish_reason_known_values() {
    assert!(matches!(map_finish_reason("STOP"), StopReason::End));
    assert!(matches!(
        map_finish_reason("MAX_TOKENS"),
        StopReason::Length
    ));
}
