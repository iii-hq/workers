mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iii_queue::adapter::QueueAdapter;
use iii_queue::adapter::SwappableAdapter;
use iii_queue::adapters::builtin::BuiltinAdapter;
use iii_queue::config::{AdapterEntry, QueueConfig};
use iii_queue::functions::{
    DLQ_MESSAGES_FN_ID, ENQUEUE_FUNCTION_FN_ID, LIST_TOPICS_FN_ID, PUBLISH_FN_ID, REDRIVE_FN_ID,
};
use iii_queue::store::FileStore;
use iii_queue::trigger::Invoker;
use iii_queue::TRIGGER_TYPE;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{IIIClient, InitOptions, RegisterFunction, TriggerAction, WorkerIdentityMode};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::sync::Notify;
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
        ..QueueConfig::default()
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
    register_subscriber_with_config(iii, function_id, queue, None);
}

fn register_subscriber_with_config(
    iii: &Arc<IIIClient>,
    function_id: &str,
    queue: &str,
    queue_config: Option<Value>,
) {
    let mut config = json!({
        "queue": queue,
        "max_retries": 1,
        "backoff_ms": 5
    });
    if let Some(qc) = queue_config {
        config["queue_config"] = qc;
    }
    iii.register_trigger(RegisterTriggerInput::new(
        TRIGGER_TYPE.to_string(),
        function_id.to_string(),
        config,
    ))
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

async fn trigger_in_namespace(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
    namespace: &str,
) -> Value {
    iii.trigger(
        TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(5_000),
        }
        .namespace(namespace),
    )
    .await
    .expect("namespaced trigger should succeed")
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

async fn wait_for_trigger_type(iii: &IIIClient, trigger_type: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let listed = iii
            .trigger(TriggerRequest {
                function_id: "engine::triggers::list".to_string(),
                payload: json!({ "prefix": trigger_type }),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        if listed.is_ok_and(|value| {
            value
                .get("triggers")
                .and_then(Value::as_array)
                .is_some_and(|triggers| {
                    triggers.iter().any(|trigger| {
                        trigger.get("id").and_then(Value::as_str) == Some(trigger_type)
                    })
                })
        }) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("trigger type '{trigger_type}' was not registered");
}

async fn wait_for_function(iii: &IIIClient, function_id: &str, namespace: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let found = iii
            .trigger(TriggerRequest {
                function_id: "engine::functions::info".to_string(),
                payload: json!({
                    "function_id": function_id,
                    "namespace": namespace,
                }),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        if found.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("function '{function_id}' was not registered");
}

async fn wait_for_registered_trigger(iii: &IIIClient, function_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let listed = iii
            .trigger(TriggerRequest {
                function_id: "engine::registered-triggers::list".to_string(),
                payload: json!({ "function_id": function_id }),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        if listed.is_ok_and(|value| {
            value
                .get("registered_triggers")
                .and_then(Value::as_array)
                .is_some_and(|triggers| !triggers.is_empty())
        }) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("trigger for function '{function_id}' was not registered");
}

async fn wait_for_subscription(adapter: &SwappableAdapter, queue: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if adapter
            .list_topics()
            .await
            .is_ok_and(|topics| topics.iter().any(|topic| topic.name == queue))
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("queue '{queue}' was not subscribed");
}

async fn wait_for_dlq(iii: &IIIClient, queue: &str, namespace: &str) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let value = trigger_in_namespace(
            iii,
            DLQ_MESSAGES_FN_ID,
            json!({
                "queue": queue,
                "limit": 50
            }),
            namespace,
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

async fn wait_for_adapter_dlq(adapter: &SwappableAdapter, queue: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if adapter.dlq_count(queue).await.unwrap_or(0) > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("message did not reach adapter DLQ");
}

#[tokio::test]
#[serial]
async fn delivery_dlq_and_redrive_connect_or_skip() {
    let suffix = Uuid::new_v4();
    let namespace = format!("queue-e2e-{suffix}");
    let Some(iii) = engine::connect_fresh_with_options(InitOptions {
        metadata: Some(WorkerMetadata {
            name: format!("queue-e2e-{suffix}"),
            ..WorkerMetadata::default()
        }),
        namespace: Some(namespace.clone()),
        identity: WorkerIdentityMode::Explicit,
        ..InitOptions::default()
    })
    .await
    else {
        return;
    };

    let dir = temp_store_dir();
    let boot = iii_queue::boot::start(iii.clone(), file_config(&dir))
        .await
        .expect("queue worker should boot");
    wait_for_trigger_type(&iii, TRIGGER_TYPE).await;

    let fires = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicBool::new(false));
    let function_id = format!("queue.e2e.{}", Uuid::new_v4());
    // Unique queue name: the engine re-delivers previously registered
    // durable:subscriber triggers on boot, so a fixed name races against
    // leftover consumers from earlier local runs.
    let queue_name = format!("e2e-demo-{}", Uuid::new_v4());
    register_counting_function(&iii, &function_id, fires.clone(), fail.clone());
    wait_for_function(&iii, &function_id, &namespace).await;
    trigger(&iii, &function_id, json!({"probe": true})).await;
    wait_for_fires(&fires, 1).await;
    fires.store(0, Ordering::SeqCst);
    register_subscriber(&iii, &function_id, &queue_name);
    wait_for_registered_trigger(&iii, &function_id).await;
    wait_for_subscription(&boot.adapter, &queue_name).await;
    let registrations = boot.trigger_handler.registrations().await;
    assert_eq!(registrations.len(), 1);
    assert_eq!(
        registrations[0].namespace.as_deref(),
        Some(namespace.as_str())
    );

    trigger(
        &iii,
        PUBLISH_FN_ID,
        json!({"queue": queue_name, "data": {"hello": "world"}}),
    )
    .await;
    wait_for_fires(&fires, 1).await;

    fail.store(true, Ordering::SeqCst);
    trigger(
        &iii,
        PUBLISH_FN_ID,
        json!({"queue": queue_name, "data": {"should": "dlq"}}),
    )
    .await;
    wait_for_adapter_dlq(&boot.adapter, &queue_name).await;
    let messages = wait_for_dlq(&iii, &queue_name, &namespace).await;
    assert_eq!(messages.len(), 1);

    fail.store(false, Ordering::SeqCst);
    trigger(&iii, REDRIVE_FN_ID, json!({"queue": queue_name})).await;
    wait_for_fires(&fires, 3).await;

    boot.shutdown().await;
    iii.shutdown_async().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
#[serial]
async fn provider_functions_and_enqueue_use_the_worker_namespace() {
    let suffix = Uuid::new_v4();
    let namespace = format!("queue-e2e-{suffix}");
    let Some(iii) = engine::connect_fresh_with_options(InitOptions {
        metadata: Some(WorkerMetadata {
            name: format!("queue-e2e-{suffix}"),
            ..WorkerMetadata::default()
        }),
        namespace: Some(namespace.clone()),
        identity: WorkerIdentityMode::Explicit,
        ..InitOptions::default()
    })
    .await
    else {
        return;
    };

    let queue_name = format!("function-queue-{suffix}");
    let mut config = QueueConfig::packaged_default();
    config
        .queue_configs
        .insert(queue_name.clone(), Default::default());
    let boot = iii_queue::boot::start(iii.clone(), config)
        .await
        .expect("queue worker should boot in its configured namespace");

    wait_for_function(&iii, LIST_TOPICS_FN_ID, &namespace).await;
    let topics = trigger_in_namespace(&iii, LIST_TOPICS_FN_ID, json!({}), &namespace).await;
    assert!(topics
        .as_array()
        .is_some_and(|topics| topics.iter().any(|topic| topic["name"] == queue_name)));

    wait_for_function(&iii, ENQUEUE_FUNCTION_FN_ID, &namespace).await;
    let fires = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicBool::new(false));
    let function_id = format!("queue::e2e::{}", suffix.simple());
    register_counting_function(&iii, &function_id, fires.clone(), fail);
    wait_for_function(&iii, &function_id, &namespace).await;

    let receipt = iii
        .trigger(
            TriggerRequest {
                function_id,
                payload: json!({"from": "enqueue-action"}),
                action: Some(TriggerAction::Enqueue {
                    queue: queue_name.clone(),
                }),
                timeout_ms: Some(5_000),
            }
            .namespace(&namespace),
        )
        .await
        .expect("namespaced enqueue action should use the queue provider");
    assert!(receipt["messageReceiptId"].is_string());
    wait_for_fires(&fires, 1).await;

    boot.shutdown().await;
    iii.shutdown_async().await;
}

struct RestartInvoker {
    started: Notify,
    resume: AtomicBool,
    fires: AtomicUsize,
}

#[async_trait]
impl Invoker for RestartInvoker {
    async fn call(&self, _function_id: &str, _payload: Value) -> Result<Option<Value>, String> {
        if self.resume.load(Ordering::SeqCst) {
            self.fires.fetch_add(1, Ordering::SeqCst);
            return Ok(None);
        }
        self.started.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
#[serial]
async fn file_based_pending_message_survives_binding_restart() {
    let dir = temp_store_dir();
    let queue_name = format!("e2e-restart-{}", Uuid::new_v4());
    let function_id = format!("queue.restart.{queue_name}");
    let subscription_id = "stable-binding";
    let invoker = Arc::new(RestartInvoker {
        started: Notify::new(),
        resume: AtomicBool::new(false),
        fires: AtomicUsize::new(0),
    });

    let store = Arc::new(FileStore::open(&dir, 5).await.unwrap());
    let adapter = BuiltinAdapter::new(store, invoker.clone());
    adapter
        .subscribe(
            &queue_name,
            subscription_id,
            &function_id,
            None,
            None,
            None,
            None,
        )
        .await;
    adapter
        .enqueue(&queue_name, json!({"survives": true}), None, None)
        .await;
    tokio::time::timeout(Duration::from_secs(3), invoker.started.notified())
        .await
        .expect("subscriber should start the blocked delivery");
    adapter.shutdown().await;
    drop(adapter);

    invoker.resume.store(true, Ordering::SeqCst);
    let store = Arc::new(FileStore::open(&dir, 5).await.unwrap());
    let adapter = BuiltinAdapter::new(store, invoker.clone());
    adapter
        .subscribe(
            &queue_name,
            subscription_id,
            &function_id,
            None,
            None,
            None,
            None,
        )
        .await;
    wait_for_fires(&invoker.fires, 1).await;

    adapter.shutdown().await;
    let _ = std::fs::remove_dir_all(dir);
}
