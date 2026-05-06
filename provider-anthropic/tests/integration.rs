//! Smoke tests for the wire format. Catches accidental field-name or tag
//! drift in the Anthropic Messages request body and the local
//! `content_block_to_wire` translator.

use harness_types::{ContentBlock, ImageContent, TextContent};
use provider_anthropic::{content_block_to_wire, AnthropicConfig, AuthMode};

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_anthropic::register_with_iii;
}

#[test]
fn request_body_round_trips_through_serde() {
    let req = serde_json::json!({
        "model": "claude-opus-4-7",
        "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}],
        "max_tokens": 100,
        "stream": true,
    });
    let serialized = serde_json::to_string(&req).expect("serializes");
    let back: serde_json::Value = serde_json::from_str(&serialized).expect("deserializes");
    assert_eq!(back["model"], "claude-opus-4-7");
    assert_eq!(back["messages"][0]["role"], "user");
    assert_eq!(back["messages"][0]["content"][0]["type"], "text");
    assert_eq!(back["max_tokens"], 100);
    assert_eq!(back["stream"], true);
}

#[test]
fn content_block_to_wire_text_round_trips() {
    let block = ContentBlock::Text(TextContent { text: "x".into() });
    let wire = content_block_to_wire(&block).expect("text serializes");
    assert_eq!(wire["type"], "text");
    assert_eq!(wire["text"], "x");
}

#[test]
fn content_block_to_wire_tool_call_round_trips() {
    let block = ContentBlock::ToolCall {
        id: "tc1".into(),
        name: "read".into(),
        arguments: serde_json::json!({"path": "/tmp/x"}),
    };
    let wire = content_block_to_wire(&block).expect("tool_call serializes");
    assert_eq!(wire["type"], "tool_use");
    assert_eq!(wire["id"], "tc1");
    assert_eq!(wire["name"], "read");
    assert_eq!(wire["input"]["path"], "/tmp/x");
}

#[test]
fn content_block_to_wire_image_yields_none() {
    // Images aren't supported in this scope; the helper returns None so the
    // caller can drop them. Locks in current behavior so a future change is
    // explicit.
    let block = ContentBlock::Image(ImageContent {
        mime: "image/png".into(),
        data: "AAAA".into(),
    });
    assert!(content_block_to_wire(&block).is_none());
}

#[test]
fn config_serde_round_trips() {
    let cfg = AnthropicConfig {
        credential_value: "sk-ant-test".into(),
        model: "claude-sonnet-4-6".into(),
        max_tokens: 4096,
        api_url: "https://api.anthropic.com/v1/messages".into(),
        auth_mode: AuthMode::OAuthBearer,
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: AnthropicConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.credential_value, cfg.credential_value);
    assert_eq!(back.model, cfg.model);
    assert!(matches!(back.auth_mode, AuthMode::OAuthBearer));
    // Drift canary: `oauth_bearer` must serialize lowercased with explicit rename.
    assert!(json.contains("\"oauth_bearer\""));
}
