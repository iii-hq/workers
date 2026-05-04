//! Smoke tests for Bedrock config shape and public consts. Bedrock's
//! request wire format is owned by the AWS SDK (pending integration), so
//! the integration test focuses on the public surface that downstream code
//! reads today.

use provider_bedrock::{BedrockConfig, DEFAULT_REGION, PROVIDER_NAME};

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_bedrock::register_with_iii;
}

#[test]
fn public_constants_match_source() {
    assert_eq!(PROVIDER_NAME, "bedrock");
    assert_eq!(DEFAULT_REGION, "us-east-1");
}

#[test]
fn config_serde_round_trips() {
    let cfg = BedrockConfig {
        model_id: "anthropic.claude-3-5-sonnet-20240620-v1:0".into(),
        region: Some(DEFAULT_REGION.into()),
        access_key_id: "AKIA".into(),
        secret_access_key: "secret".into(),
        max_tokens: 4096,
    };
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: BedrockConfig = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.model_id, "anthropic.claude-3-5-sonnet-20240620-v1:0");
    assert_eq!(back.region.as_deref(), Some(DEFAULT_REGION));
    assert_eq!(back.access_key_id, "AKIA");
}

#[test]
fn config_deserializes_with_missing_credential_fields() {
    // `access_key_id` and `secret_access_key` carry `#[serde(default)]` so
    // legacy `from_env()` configs (which leave them empty) round-trip
    // cleanly. Lock that in here.
    let json = serde_json::json!({
        "model_id": "anthropic.claude-3-5-sonnet-20240620-v1:0",
        "region": null,
        "max_tokens": 4096,
    });
    let cfg: BedrockConfig = serde_json::from_value(json).expect("deserializes");
    assert_eq!(cfg.access_key_id, "");
    assert_eq!(cfg.secret_access_key, "");
    assert!(cfg.region.is_none());
}

#[test]
fn with_credential_api_key_populates_access_key_id() {
    // AWS_SECRET_ACCESS_KEY + AWS_REGION are required env reads; assert the
    // failure mode when they're absent so the contract stays explicit.
    let prev_secret = std::env::var("AWS_SECRET_ACCESS_KEY").ok();
    let prev_region = std::env::var("AWS_REGION").ok();
    std::env::remove_var("AWS_SECRET_ACCESS_KEY");
    std::env::remove_var("AWS_REGION");

    let cred = auth_credentials::Credential::ApiKey { key: "AKIA".into() };
    let err = BedrockConfig::with_credential("anthropic.claude-3-5-sonnet", &cred).unwrap_err();
    assert!(err.to_string().contains("AWS_SECRET_ACCESS_KEY"));

    if let Some(v) = prev_secret {
        std::env::set_var("AWS_SECRET_ACCESS_KEY", v);
    }
    if let Some(v) = prev_region {
        std::env::set_var("AWS_REGION", v);
    }
}
