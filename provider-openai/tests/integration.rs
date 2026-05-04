//! Smoke tests for OpenAI Chat Completions wire shape + public consts.

use provider_openai::{OpenAIConfig, DEFAULT_API_URL, PROVIDER_NAME};

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_openai::register_with_iii;
}

#[test]
fn public_constants_match_source() {
    // Drift canary: a future rename of these consts (or their values) will
    // surface here before it reaches a downstream registry build.
    assert_eq!(PROVIDER_NAME, "openai");
    assert_eq!(
        DEFAULT_API_URL,
        "https://api.openai.com/v1/chat/completions"
    );
}

#[test]
fn config_serde_round_trips() {
    let cfg = OpenAIConfig {
        api_key: "sk-test".into(),
        model: "gpt-4o".into(),
        max_tokens: 4096,
        api_url: DEFAULT_API_URL.into(),
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: OpenAIConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.api_key, "sk-test");
    assert_eq!(back.model, "gpt-4o");
    assert_eq!(back.api_url, DEFAULT_API_URL);
}

#[test]
fn with_credential_oauth_uses_access_token() {
    let cred = auth_credentials::Credential::OAuth {
        access_token: "tok".into(),
        refresh_token: None,
        expires_at: None,
        scopes: vec![],
        provider_extra: serde_json::Value::Null,
    };
    let cfg = OpenAIConfig::with_credential("gpt-4o", &cred).expect("builds");
    assert_eq!(cfg.api_key, "tok");
    assert_eq!(cfg.api_url, DEFAULT_API_URL);
}

#[test]
fn chat_completions_request_round_trips() {
    let req = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 100,
        "stream": true,
    });
    let serialized = serde_json::to_string(&req).expect("serializes");
    let back: serde_json::Value = serde_json::from_str(&serialized).expect("deserializes");
    assert_eq!(back["model"], "gpt-4o");
    assert_eq!(back["messages"][0]["role"], "user");
    assert_eq!(back["stream"], true);
}
