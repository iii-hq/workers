use judge_laya::{configuration, LayaConfig};
use serde_json::json;

#[test]
fn config_validates_model_revision_and_limits() {
    let defaults = LayaConfig::default().to_json();
    assert_eq!(defaults["model"], "laya");
    assert_eq!(defaults["batch_questions"], 16);
    assert!((1..=8).contains(&defaults["threads"].as_u64().unwrap()));
    assert!(defaults.get("revision").is_none());
    assert_eq!(defaults["preload"], json!([]));
    assert_eq!(defaults["auto_route"], false);
    assert_eq!(defaults["auto_task_detection"], false);
    assert!(defaults.get("shortlist_k").is_none());
    let routed = LayaConfig::from_json(&json!({
        "preload": ["laya-multilingual", "laya-typed-decisions"],
        "auto_route": true,
        "auto_task_detection": true,
        "shortlist_k": 20
    }))
    .unwrap();
    assert!(routed.routing().auto_route && routed.routing().auto_task_detection);
    assert_eq!(routed.routing().shortlist_k, Some(20));
    let ok = LayaConfig::from_json(
        &json!({"model": "laya-multilingual", "revision": "abc123", "batch_questions": 4}),
    )
    .unwrap();
    assert_eq!(ok.limits().batch_questions, 4);
    for bad in [
        json!({"model": "jev"}),
        json!({"revision": " "}),
        json!({"revision": "../x"}),
        json!({"batch_questions": 0}),
        json!({"threads": 0}),
        json!({"threads": 257}),
        json!({"batch_questions": 257}),
        json!({"max_timeout_ms": 0}),
        json!({"api_key": "test-marker"}),
        json!({"preload": ["laya"]}),
        json!({"preload": ["laya-multilingual", "laya-multilingual"]}),
        json!({"preload": ["jev"]}),
        json!({"shortlist_k": 0}),
        json!({"shortlist_k": 257}),
    ] {
        assert!(LayaConfig::from_json(&bad).is_err(), "{bad}");
    }
    let schema = LayaConfig::json_schema();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["model"]["enum"],
        json!(["laya", "laya-multilingual", "laya-typed-decisions"])
    );
    assert_eq!(
        schema["properties"]["preload"]["items"]["enum"],
        schema["properties"]["model"]["enum"]
    );
}

#[tokio::test]
async fn apply_keeps_the_last_valid_snapshot() {
    let cell = configuration::new_cell(LayaConfig::default());
    assert!(
        configuration::apply_config(
            &cell,
            LayaConfig {
                batch_questions: 2,
                ..LayaConfig::default()
            }
        )
        .await
    );
    assert!(
        !configuration::apply_config(
            &cell,
            LayaConfig {
                model: "nope".into(),
                ..LayaConfig::default()
            }
        )
        .await
    );
    assert_eq!(cell.read().await.batch_questions, 2);
}
