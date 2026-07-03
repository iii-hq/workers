mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_queue::config::{AdapterEntry, QueueConfig};
use iii_queue::functions::{DLQ_MESSAGES_FN_ID, PUBLISH_FN_ID, REDRIVE_FN_ID};
use iii_queue::TRIGGER_TYPE;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::time::Instant;
use uuid::Uuid;

use common::engine;

fn temp_store_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("queue_e2e_{}", Uuid::new_v4()))
}

fn file_config(dir: &std::path::Path) -> QueueConfig {
    QueueConfig {
        adapter: Some(AdapterEntry {
            name: "builtin".to_string(),
            config: Some(json!({
                "store_method": "file_based",
                "file_path": dir.to_string_lossy(),
                "save_interval_ms": 5
            })),
        }),
    }
}

fn register_counting_function(
    iii: &Arc<IIIClient>,
    function_id: &str,
    fires: Arc<AtomicUsize>,
    fail: Arc<AtomicBool>,
) {
    iii.register_function(
        function_id,
        RegisterFunction::new_async(move |_payload: Value| {
            let fires = fires.clone();
            let fail = fail.clone();
            async move {
                fires.fetch_add(1, Ordering::SeqCst);
                if fail.load(Ordering::SeqCst) {
                    Err(Error::Handler("e2e forced failure".to_string()))
                } else {
                    Ok::<Value, Error>(json!({"ok": true}))
                }
            }
        }),
    );
}

fn register_subscriber(iii: &Arc<IIIClient>, function_id: &str, queue: &str) {
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: TRIGGER_TYPE.to_string(),
        function_id: function_id.to_string(),
        config: json!({
            "queue": queue,
            "max_retries": 1,
            "backoff_ms": 5
        }),
        metadata: None,
    })
    .expect("register durable subscriber trigger");
}

async fn trigger(iii: &IIIClient, function_id: &str, payload: Value) -> Value {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
    .expect("trigger should succeed")
}

async fn wait_for_fires(fires: &AtomicUsize, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if fires.load(Ordering::SeqCst) >= expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        fires.load(Ordering::SeqCst) >= expected,
        "expected at least {expected} fires"
    );
}

async fn wait_for_dlq(iii: &IIIClient, queue: &str) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let value = trigger(
            iii,
            DLQ_MESSAGES_FN_ID,
            json!({
                "queue": queue,
                "limit": 50
            }),
        )
        .await;
        let messages = serde_json::from_value::<Vec<Value>>(value).unwrap_or_default();
        if !messages.is_empty() {
            return messages;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("message did not reach DLQ");
}

#[tokio::test]
#[serial]
async fn delivery_dlq_and_redrive_connect_or_skip() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };

    let dir = temp_store_dir();
    let boot = iii_queue::boot::start(iii.clone(), file_config(&dir))
        .await
        .expect("queue worker should boot");

    let fires = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicBool::new(false));
    let function_id = format!("queue.e2e.{}", Uuid::new_v4());
    register_counting_function(&iii, &function_id, fires.clone(), fail.clone());
    register_subscriber(&iii, &function_id, "demo");

    trigger(
        &iii,
        PUBLISH_FN_ID,
        json!({"queue": "demo", "data": {"hello": "world"}}),
    )
    .await;
    wait_for_fires(&fires, 1).await;

    fail.store(true, Ordering::SeqCst);
    trigger(
        &iii,
        PUBLISH_FN_ID,
        json!({"queue": "demo", "data": {"should": "dlq"}}),
    )
    .await;
    let messages = wait_for_dlq(&iii, "demo").await;
    assert_eq!(messages.len(), 1);

    fail.store(false, Ordering::SeqCst);
    trigger(&iii, REDRIVE_FN_ID, json!({"queue": "demo"})).await;
    wait_for_fires(&fires, 3).await;

    boot.shutdown().await;
    iii.shutdown_async().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
#[serial]
async fn file_based_pending_message_survives_worker_restart_connect_or_skip() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };

    let dir = temp_store_dir();
    let boot = iii_queue::boot::start(iii.clone(), file_config(&dir))
        .await
        .expect("queue worker should boot");

    trigger(
        &iii,
        PUBLISH_FN_ID,
        json!({"queue": "restart", "data": {"survives": true}}),
    )
    .await;
    boot.shutdown().await;

    let boot = iii_queue::boot::start(iii.clone(), file_config(&dir))
        .await
        .expect("queue worker should reboot");

    let fires = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicBool::new(false));
    let function_id = format!("queue.restart.{}", Uuid::new_v4());
    register_counting_function(&iii, &function_id, fires.clone(), fail);
    register_subscriber(&iii, &function_id, "restart");
    wait_for_fires(&fires, 1).await;

    boot.shutdown().await;
    iii.shutdown_async().await;
    let _ = std::fs::remove_dir_all(dir);
}
