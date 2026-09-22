use judge_semif::{configuration, SemifConfig};
use serde_json::json;

#[test]
fn config_validates_model_placement_and_limits() {
    let defaults = SemifConfig::default().to_json();
    assert_eq!(defaults["model"], "qwen3.5-4b");
    assert_eq!(defaults["context_tokens"], 16384);
    assert!((1..=8).contains(&defaults["threads"].as_u64().unwrap()));
    assert!(defaults.get("gpu_layers").is_none());
    let ok = SemifConfig::from_json(&json!({"gpu_layers": 0, "context_tokens": 4096})).unwrap();
    assert_eq!((ok.gpu_layers, ok.context_tokens), (Some(0), 4096));
    for bad in [
        json!({"model": "laya"}),
        json!({"threads": 0}),
        json!({"threads": 257}),
        json!({"context_tokens": 100}),
        json!({"max_timeout_ms": 0}),
        json!({"api_key": "test-marker"}),
    ] {
        assert!(SemifConfig::from_json(&bad).is_err(), "{bad}");
    }
    let schema = SemifConfig::json_schema();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["model"]["enum"], json!(["qwen3.5-4b"]));
}

#[tokio::test]
async fn apply_keeps_the_last_valid_snapshot() {
    let cell = configuration::new_cell(SemifConfig::default());
    assert!(
        configuration::apply_config(
            &cell,
            SemifConfig {
                max_timeout_ms: 5000,
                ..SemifConfig::default()
            }
        )
        .await
    );
    assert!(
        !configuration::apply_config(
            &cell,
            SemifConfig {
                model: "nope".into(),
                ..SemifConfig::default()
            }
        )
        .await
    );
    assert_eq!(cell.read().await.max_timeout_ms, 5000);
}
