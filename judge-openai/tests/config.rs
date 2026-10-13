use judge_openai::config::OpenAiConfig;
use serde_json::json;

#[test]
fn execution_limits_accept_positive_configuration_with_documented_defaults() {
    let configured = OpenAiConfig::from_json(
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
    let defaults = OpenAiConfig::default().to_json();
    assert_eq!(defaults["max_request_bytes"], 8388608);
    assert_eq!(defaults["max_response_bytes"], 8388608);
    assert_eq!(defaults["max_timeout_ms"], 300000);
    for field in ["max_request_bytes", "max_response_bytes", "max_timeout_ms"] {
        assert_eq!(
            OpenAiConfig::json_schema()["properties"][field]["minimum"].as_f64(),
            Some(1.0)
        );
        for value in [json!(0), json!(-1), json!(1.5), json!("1024"), json!(null)] {
            assert!(OpenAiConfig::from_json(&json!({field:value})).is_err());
        }
    }
}

#[test]
fn config_roundtrips_credentials_but_debug_and_errors_are_redacted() {
    let config =
        OpenAiConfig::from_json(&json!({"api_key":"test-config-marker","model":"gpt-6-luna"}))
            .unwrap();
    assert_eq!(config.to_json()["api_key"], "test-config-marker");
    assert!(!format!("{config:?}").contains("test-config-marker"));
    for bad in [
        json!({"api_key":{"test-config-marker":1}}),
        json!({"test-config-marker":true}),
        json!({"model":"  "}),
        json!({"model":"gpt-5.6-luna"}),
    ] {
        let message = OpenAiConfig::from_json(&bad).unwrap_err();
        assert!(!message.contains("test-config-marker"));
    }
}
#[test]
fn config_schema_exposes_password_without_accepting_a_provider_url() {
    let schema = OpenAiConfig::json_schema();
    assert_eq!(schema["properties"]["api_key"]["writeOnly"], true);
    assert_eq!(schema["properties"]["api_key"]["format"], "password");
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["model"]["enum"], json!(["gpt-6-luna"]));
    assert_eq!(OpenAiConfig::default().model, "gpt-6-luna");
    assert!(OpenAiConfig::from_json(&json!({"endpoint":"http://127.0.0.1"})).is_err());
}
#[tokio::test]
async fn apply_replaces_snapshot_and_rejects_unsupported_models() {
    let cell = judge_openai::configuration::new_cell(OpenAiConfig::default());
    let old = cell.read().await.clone();
    assert!(
        judge_openai::configuration::apply_config(
            &cell,
            OpenAiConfig {
                api_key: Some("next-test-marker".into()),
                max_timeout_ms: 1234,
                ..OpenAiConfig::default()
            }
        )
        .await
    );
    assert_eq!(old.max_timeout_ms, 300_000);
    assert_eq!(cell.read().await.max_timeout_ms, 1234);
    assert!(
        !judge_openai::configuration::apply_config(
            &cell,
            OpenAiConfig {
                api_key: None,
                model: "gpt-5.6-luna".into(),
                ..OpenAiConfig::default()
            }
        )
        .await
    );
    assert_eq!(cell.read().await.max_timeout_ms, 1234);
}
