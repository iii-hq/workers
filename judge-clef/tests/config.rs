use judge_clef::{configuration, ClefConfig};
use serde_json::json;

#[test]
fn config_validates_model_placement_and_limits() {
    let defaults = ClefConfig::default().to_json();
    assert_eq!(defaults["model"], "clef-flash");
    assert_eq!(defaults["context_tokens"], 16384);
    assert!((1..=8).contains(&defaults["threads"].as_u64().unwrap()));
    assert!(defaults.get("gpu_layers").is_none());
    let ok = ClefConfig::from_json(&json!({"gpu_layers": 0, "context_tokens": 16384})).unwrap();
    assert_eq!((ok.gpu_layers, ok.context_tokens), (Some(0), 16384));
    assert_eq!(
        ClefConfig::from_json(&json!({"context_tokens": 512}))
            .unwrap()
            .context_tokens,
        512
    );
    for bad in [
        json!({"model": "clef"}),
        json!({"threads": 0}),
        json!({"threads": 257}),
        json!({"context_tokens": 511}),
        // One pass over 16384 tokens is the most measured to fit a GPU safely.
        json!({"context_tokens": 16385}),
        // Clef decides every question in one pass: no per-question batching.
        json!({"parallel_questions": 4}),
        json!({"max_request_bytes": 0}),
        json!({"max_timeout_ms": 0}),
        json!({"api_key": "test-marker"}),
    ] {
        assert!(ClefConfig::from_json(&bad).is_err(), "{bad}");
    }
    let schema = ClefConfig::json_schema();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["model"]["enum"], json!(["clef-flash"]));
    assert!(schema["properties"].get("parallel_questions").is_none());
}

#[tokio::test]
async fn apply_keeps_the_last_valid_snapshot() {
    let cell = configuration::new_cell(ClefConfig::default());
    assert!(
        configuration::apply_config(
            &cell,
            ClefConfig {
                max_timeout_ms: 5000,
                ..ClefConfig::default()
            }
        )
        .await
    );
    assert!(
        !configuration::apply_config(
            &cell,
            ClefConfig {
                model: "nope".into(),
                ..ClefConfig::default()
            }
        )
        .await
    );
    assert_eq!(cell.read().await.max_timeout_ms, 5000);
}
