//! Smoke tests for deepseek config + Chat Completions request shape.

use provider_deepseek::DeepSeekConfig;

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_deepseek::register_with_iii;
}

#[test]
fn config_serde_round_trips() {
    let cfg = DeepSeekConfig {
        api_key: "sk-test".into(),
        model: "deepseek-chat".into(),
        max_tokens: 4096,
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: DeepSeekConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.api_key, "sk-test");
    assert_eq!(back.model, "deepseek-chat");
    assert_eq!(back.max_tokens, 4096);
}

#[test]
fn with_credential_api_key_populates_config() {
    let cred = auth_credentials::Credential::ApiKey {
        key: "sk-key".into(),
    };
    let cfg = DeepSeekConfig::with_credential("deepseek-chat", &cred).expect("builds");
    assert_eq!(cfg.api_key, "sk-key");
    assert_eq!(cfg.model, "deepseek-chat");
}

#[test]
fn chat_completions_request_round_trips() {
    let req = serde_json::json!({
        "model": "deepseek-chat",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 100,
        "stream": true,
    });
    let serialized = serde_json::to_string(&req).expect("serializes");
    let back: serde_json::Value = serde_json::from_str(&serialized).expect("deserializes");
    assert_eq!(back["model"], "deepseek-chat");
    assert_eq!(back["messages"][0]["role"], "user");
    assert_eq!(back["stream"], true);
}
