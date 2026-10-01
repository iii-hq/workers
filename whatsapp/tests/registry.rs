use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;
use whatsapp::{config::Config, registry::Registry, sessions::SessionManager};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Input {
    value: u32,
}
#[derive(Serialize, JsonSchema)]
struct Output {
    next: u32,
}

#[tokio::test]
async fn typed_registry_checks_requests_preserves_errors_and_accepts_engine_metadata() {
    let temporary = TempDir::new().unwrap();
    let manager = SessionManager::open(Config {
        data_dir: temporary.path().join("sessions"),
        ..Config::default()
    })
    .unwrap();
    let mut registry = Registry::default();
    registry.add::<Input, Output, _, _>(
        "whatsapp::test",
        "checked transformation",
        |_, input| async move {
            Ok(Output {
                next: input
                    .value
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("overflow"))?,
            })
        },
    );
    assert_eq!(
        registry
            .invoke(
                "whatsapp::test",
                manager.clone(),
                json!({"value": 7, "_caller_worker_id":"engine"})
            )
            .await
            .unwrap(),
        json!({"next":8})
    );
    for input in [
        json!({"value": -1}),
        json!({"value":8, "typo":true}),
        json!({}),
    ] {
        assert!(
            registry
                .invoke("whatsapp::test", manager.clone(), input)
                .await
                .is_err()
        );
    }
    assert!(
        registry
            .invoke("whatsapp::test", manager.clone(), json!({"value":u32::MAX}))
            .await
            .unwrap_err()
            .to_string()
            .contains("overflow")
    );
    assert!(
        registry
            .invoke("whatsapp::missing", manager, json!({}))
            .await
            .is_err()
    );
    let catalog = registry.catalog();
    assert_eq!(catalog[0].request_format["additionalProperties"], false);
    assert_eq!(
        catalog[0].response_format["properties"]["next"]["type"],
        "integer"
    );
}

#[tokio::test]
async fn timeout_explains_unknown_remote_outcome_without_retrying() {
    let temporary = TempDir::new().unwrap();
    let manager = SessionManager::open(Config {
        data_dir: temporary.path().join("sessions"),
        operation_timeout_secs: 1,
        ..Config::default()
    })
    .unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let mut registry = Registry::default();
    registry.add::<Input, Output, _, _>("whatsapp::slow", "never completes", move |_, _| {
        counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        async { std::future::pending::<Result<Output>>().await }
    });
    let error = registry
        .invoke("whatsapp::slow", manager, json!({"value":1}))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("may have completed"));
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
}
