use jev::config::JevConfig;
use serde_json::json;

#[test]
fn execution_limits_accept_positive_configuration_with_documented_defaults() {
    let configured = JevConfig::from_json(
        &json!({"max_request_bytes":1024,"max_response_bytes":2048,"max_timeout_ms":90000}),
    );
    assert!(
        configured.is_ok(),
        "operator limits must be configurable: {configured:?}"
    );
    let configured = configured.unwrap().to_json();
    assert_eq!(configured["max_request_bytes"], 1024);
    assert_eq!(configured["max_response_bytes"], 2048);
    assert_eq!(configured["max_timeout_ms"], 90000);
    let defaults = JevConfig::default().to_json();
    assert_eq!(defaults["max_request_bytes"], 8388608);
    assert_eq!(defaults["max_response_bytes"], 8388608);
    assert_eq!(defaults["max_timeout_ms"], 300000);
    for field in ["max_request_bytes", "max_response_bytes", "max_timeout_ms"] {
        assert_eq!(
            JevConfig::json_schema()["properties"][field]["minimum"].as_f64(),
            Some(1.0)
        );
        for value in [json!(0), json!(-1), json!(1.5), json!("1024"), json!(null)] {
            assert!(JevConfig::from_json(&json!({field:value})).is_err());
        }
    }
}

#[test]
fn config_roundtrips_credentials_but_debug_and_errors_are_redacted() {
    let config =
        JevConfig::from_json(&json!({"api_key":"test-config-marker","model":"jev-1.13.0"}))
            .unwrap();
    assert_eq!(config.to_json()["api_key"], "test-config-marker");
    assert!(!format!("{config:?}").contains("test-config-marker"));
    for bad in [
        json!({"api_key":{"test-config-marker":1}}),
        json!({"test-config-marker":true}),
        json!({"model":"  "}),
    ] {
        let message = JevConfig::from_json(&bad).unwrap_err();
        assert!(!message.contains("test-config-marker"));
    }
}
#[test]
fn config_schema_exposes_password_without_accepting_a_provider_url() {
    let schema = JevConfig::json_schema();
    assert_eq!(schema["properties"]["api_key"]["writeOnly"], true);
    assert_eq!(schema["properties"]["api_key"]["format"], "password");
    assert_eq!(schema["additionalProperties"], false);
    assert!(JevConfig::from_json(&json!({"endpoint":"http://127.0.0.1"})).is_err());
}
#[tokio::test]
async fn apply_replaces_snapshot_and_rejects_invalid_model() {
    let cell = jev::configuration::new_cell(JevConfig::default());
    let old = cell.read().await.clone();
    assert!(
        jev::configuration::apply_config(
            &cell,
            JevConfig {
                api_key: Some("next-test-marker".into()),
                model: "another-model".into(),
                ..JevConfig::default()
            }
        )
        .await
    );
    assert_eq!(old.model, "jev-1.13.0");
    assert_eq!(cell.read().await.model, "another-model");
    assert!(
        !jev::configuration::apply_config(
            &cell,
            JevConfig {
                api_key: None,
                model: " ".into(),
                ..JevConfig::default()
            }
        )
        .await
    );
    assert_eq!(cell.read().await.model, "another-model");
}
