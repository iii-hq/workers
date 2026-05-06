//! Smoke tests for huggingface config + Chat Completions request shape.

use provider_huggingface::HuggingFaceConfig;

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_huggingface::register_with_iii;
}

#[test]
fn config_serde_round_trips() {
    let cfg = HuggingFaceConfig {
        api_key: "hf_test".into(),
        model: "meta-llama/Llama-3.1-70B-Instruct".into(),
        max_tokens: 4096,
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: HuggingFaceConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.api_key, "hf_test");
    assert_eq!(back.model, "meta-llama/Llama-3.1-70B-Instruct");
    assert_eq!(back.max_tokens, 4096);
}

#[test]
fn with_credential_api_key_populates_config() {
    let cred = auth_credentials::Credential::ApiKey {
        key: "hf_key".into(),
    };
    let cfg = HuggingFaceConfig::with_credential("meta-llama/Llama-3.1-70B-Instruct", &cred)
        .expect("builds");
    assert_eq!(cfg.api_key, "hf_key");
    assert_eq!(cfg.model, "meta-llama/Llama-3.1-70B-Instruct");
}

#[test]
fn chat_completions_request_round_trips() {
    let req = serde_json::json!({
        "model": "meta-llama/Llama-3.1-70B-Instruct",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 100,
        "stream": true,
    });
    let serialized = serde_json::to_string(&req).expect("serializes");
    let back: serde_json::Value = serde_json::from_str(&serialized).expect("deserializes");
    assert_eq!(back["model"], "meta-llama/Llama-3.1-70B-Instruct");
    assert_eq!(back["messages"][0]["role"], "user");
    assert_eq!(back["stream"], true);
}
