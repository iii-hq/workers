use judge::{configuration, JudgeConfig};
use serde_json::json;

#[test]
fn provider_is_validated_as_a_worker_name_suffix_and_unknown_fields_are_rejected() {
    assert_eq!(
        JudgeConfig::default().to_json(),
        json!({"provider": "typesafe"})
    );
    assert_eq!(
        JudgeConfig::from_json(&json!({"provider": "local-llm"}))
            .unwrap()
            .provider,
        "local-llm"
    );
    for bad in [
        json!({"provider": ""}),
        json!({"provider": "A"}),
        json!({"provider": "a::b"}),
        json!({"provider": "x".repeat(65)}),
        json!({"provider": 3}),
        json!({"api_key": "test-marker"}),
    ] {
        assert!(JudgeConfig::from_json(&bad).is_err(), "{bad}");
    }
    let schema = JudgeConfig::json_schema();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["provider"]["pattern"],
        "^[a-z0-9-]{1,64}$"
    );
}

#[tokio::test]
async fn apply_replaces_the_snapshot_and_keeps_it_on_invalid_reloads() {
    let cell = configuration::new_cell(JudgeConfig::default());
    assert!(
        configuration::apply_config(
            &cell,
            JudgeConfig {
                provider: "local-llm".into(),
                preload_all: false,
            }
        )
        .await
    );
    assert_eq!(cell.read().await.provider, "local-llm");
    assert!(
        !configuration::apply_config(
            &cell,
            JudgeConfig {
                provider: "Bad Name".into(),
                preload_all: false,
            }
        )
        .await
    );
    assert_eq!(cell.read().await.provider, "local-llm");
}

#[test]
fn preload_all_is_off_by_default_and_round_trips() {
    assert!(!JudgeConfig::default().preload_all);
    // Off is not stored: the seeded entry stays `{"provider": ...}`.
    assert!(JudgeConfig::default()
        .to_json()
        .get("preload_all")
        .is_none());
    let config =
        JudgeConfig::from_json(&serde_json::json!({"provider": "laya", "preload_all": true}))
            .unwrap();
    assert!(config.preload_all);
    assert_eq!(config.to_json()["preload_all"], true);
    assert_eq!(
        JudgeConfig::json_schema()["properties"]["preload_all"]["type"],
        "boolean"
    );
}
