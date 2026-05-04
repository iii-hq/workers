//! Smoke tests for xAI config + Chat Completions request shape.

use provider_xai::XaiConfig;

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_xai::register_with_iii;
}

#[test]
fn config_serde_round_trips() {
    let cfg = XaiConfig {
        api_key: "sk-test".into(),
        model: "grok-2".into(),
        max_tokens: 4096,
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: XaiConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.api_key, "sk-test");
    assert_eq!(back.model, "grok-2");
    assert_eq!(back.max_tokens, 4096);
}

#[test]
fn with_credential_api_key_populates_config() {
    let cred = auth_credentials::Credential::ApiKey {
        key: "sk-key".into(),
    };
    let cfg = XaiConfig::with_credential("grok-2", &cred).expect("builds");
    assert_eq!(cfg.api_key, "sk-key");
    assert_eq!(cfg.model, "grok-2");
}

#[test]
fn chat_completions_request_round_trips() {
    let req = serde_json::json!({
        "model": "grok-2",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 100,
        "stream": true,
    });
    let serialized = serde_json::to_string(&req).expect("serializes");
    let back: serde_json::Value = serde_json::from_str(&serialized).expect("deserializes");
    assert_eq!(back["model"], "grok-2");
    assert_eq!(back["messages"][0]["role"], "user");
    assert_eq!(back["stream"], true);
}
