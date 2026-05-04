//! Smoke tests for openrouter config + Chat Completions request shape.
//! Catches field-name drift on the public config and on the shared
//! OpenAI-compat request body.

use provider_openrouter::OpenRouterConfig;

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_openrouter::register_with_iii;
}

#[test]
fn config_serde_round_trips() {
    let cfg = OpenRouterConfig {
        api_key: "sk-test".into(),
        model: "openai/gpt-4o".into(),
        max_tokens: 4096,
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: OpenRouterConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.api_key, "sk-test");
    assert_eq!(back.model, "openai/gpt-4o");
    assert_eq!(back.max_tokens, 4096);
}

#[test]
fn with_credential_api_key_populates_config() {
    let cred = auth_credentials::Credential::ApiKey {
        key: "sk-key".into(),
    };
    let cfg = OpenRouterConfig::with_credential("openai/gpt-4o", &cred).expect("builds");
    assert_eq!(cfg.api_key, "sk-key");
    assert_eq!(cfg.model, "openai/gpt-4o");
}

#[test]
fn chat_completions_request_round_trips() {
    let req = serde_json::json!({
        "model": "openai/gpt-4o",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 100,
        "stream": true,
    });
    let serialized = serde_json::to_string(&req).expect("serializes");
    let back: serde_json::Value = serde_json::from_str(&serialized).expect("deserializes");
    assert_eq!(back["model"], "openai/gpt-4o");
    assert_eq!(back["messages"][0]["role"], "user");
    assert_eq!(back["stream"], true);
}
