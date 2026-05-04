//! Smoke tests for mistral config + Chat Completions request shape.

use provider_mistral::MistralConfig;

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_mistral::register_with_iii;
}

#[test]
fn config_serde_round_trips() {
    let cfg = MistralConfig {
        api_key: "sk-test".into(),
        model: "mistral-large-latest".into(),
        max_tokens: 4096,
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: MistralConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.api_key, "sk-test");
    assert_eq!(back.model, "mistral-large-latest");
    assert_eq!(back.max_tokens, 4096);
}

#[test]
fn with_credential_api_key_populates_config() {
    let cred = auth_credentials::Credential::ApiKey {
        key: "sk-key".into(),
    };
    let cfg = MistralConfig::with_credential("mistral-large-latest", &cred).expect("builds");
    assert_eq!(cfg.api_key, "sk-key");
    assert_eq!(cfg.model, "mistral-large-latest");
}

#[test]
fn chat_completions_request_round_trips() {
    let req = serde_json::json!({
        "model": "mistral-large-latest",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 100,
        "stream": true,
    });
    let serialized = serde_json::to_string(&req).expect("serializes");
    let back: serde_json::Value = serde_json::from_str(&serialized).expect("deserializes");
    assert_eq!(back["model"], "mistral-large-latest");
    assert_eq!(back["messages"][0]["role"], "user");
    assert_eq!(back["stream"], true);
}
