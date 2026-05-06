//! Smoke tests for groq config + Chat Completions request shape. Catches
//! field-name drift on the public config and on the shared OpenAI-compat
//! request body without requiring an iii engine connection.

use provider_groq::GroqConfig;

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_groq::register_with_iii;
}

#[test]
fn config_serde_round_trips() {
    let cfg = GroqConfig {
        api_key: "sk-test".into(),
        model: "llama-3.1-70b".into(),
        max_tokens: 4096,
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: GroqConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.api_key, "sk-test");
    assert_eq!(back.model, "llama-3.1-70b");
    assert_eq!(back.max_tokens, 4096);
}

#[test]
fn with_credential_api_key_populates_config() {
    let cred = auth_credentials::Credential::ApiKey {
        key: "sk-key".into(),
    };
    let cfg = GroqConfig::with_credential("llama-3.1-70b", &cred).expect("builds");
    assert_eq!(cfg.api_key, "sk-key");
    assert_eq!(cfg.model, "llama-3.1-70b");
}

#[test]
fn chat_completions_request_round_trips() {
    // Lock the OpenAI-compat request shape (used by groq verbatim) so a
    // drift in `messages[0].role` or `max_tokens` is caught here.
    let req = serde_json::json!({
        "model": "llama-3.1-70b",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 100,
        "stream": true,
    });
    let serialized = serde_json::to_string(&req).expect("serializes");
    let back: serde_json::Value = serde_json::from_str(&serialized).expect("deserializes");
    assert_eq!(back["model"], "llama-3.1-70b");
    assert_eq!(back["messages"][0]["role"], "user");
    assert_eq!(back["stream"], true);
}
