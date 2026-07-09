mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_queue::config::{AdapterEntry, QueueConfig};
use iii_queue::functions::{DLQ_MESSAGES_FN_ID, LIST_TOPICS_FN_ID, PUBLISH_FN_ID, REDRIVE_FN_ID};
use iii_queue::TRIGGER_TYPE;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::sync::{Barrier, Mutex};
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
        ..Default::default()
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
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: TRIGGER_TYPE.to_string(),
        function_id: function_id.to_string(),
        config,
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

async fn wait_for_topics(iii: &IIIClient, expected: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let value = trigger(iii, LIST_TOPICS_FN_ID, json!({})).await;
        let names = value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|topic| topic.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        if expected.iter().all(|topic| names.contains(topic)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("queue topics were not restored: {expected:?}");
}

#[tokio::test]
#[serial]
async fn saturated_subagent_topic_does_not_block_root_delivery_connect_or_skip() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let dir = temp_store_dir();
    let boot = iii_queue::boot::start(iii.clone(), file_config(&dir))
        .await
        .expect("queue worker should boot");

    let root_topic = format!("harness-turn-{}", Uuid::new_v4());
    let subagent_topic = format!("harness-subagent-{}", Uuid::new_v4());
    let reactive_topic = format!("harness-reactive-{}", Uuid::new_v4());
    let function_id = format!("queue.lanes.{}", Uuid::new_v4());
    let subagents_started = Arc::new(AtomicUsize::new(0));
    let roots_delivered = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(Barrier::new(11));

    iii.register_function(
        &function_id,
        RegisterFunction::new_async({
            let subagents_started = subagents_started.clone();
            let roots_delivered = roots_delivered.clone();
            let release = release.clone();
            move |payload: Value| {
                let subagents_started = subagents_started.clone();
                let roots_delivered = roots_delivered.clone();
                let release = release.clone();
                async move {
                    if payload["lane"] == "subagent" {
                        subagents_started.fetch_add(1, Ordering::SeqCst);
                        release.wait().await;
                    } else if payload["lane"] == "root" {
                        roots_delivered.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok::<Value, Error>(json!({"ok": true}))
                }
            }
        }),
    );
    for topic in [&root_topic, &subagent_topic, &reactive_topic] {
        register_subscriber_with_config(
            &iii,
            &function_id,
            topic,
            Some(json!({"type": "standard", "concurrency": 10})),
        );
    }
    wait_for_topics(&iii, &[&root_topic, &subagent_topic, &reactive_topic]).await;

    for _ in 0..10 {
        trigger(
            &iii,
            PUBLISH_FN_ID,
            json!({"queue": subagent_topic, "data": {"lane": "subagent"}}),
        )
        .await;
    }
    wait_for_fires(&subagents_started, 10).await;

    trigger(
        &iii,
        PUBLISH_FN_ID,
        json!({"queue": root_topic, "data": {"lane": "root"}}),
    )
    .await;
    wait_for_fires(&roots_delivered, 1).await;

    release.wait().await;
    boot.shutdown().await;
    iii.shutdown_async().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
#[serial]
async fn queue_restart_replays_three_topics_without_duplicate_delivery_connect_or_skip() {
    let Some(queue_client) = engine::connect_fresh().await else {
        return;
    };
    let Some(harness_client) = engine::connect_fresh().await else {
        return;
    };
    let dir = temp_store_dir();
    let boot = iii_queue::boot::start(queue_client.clone(), file_config(&dir))
        .await
        .expect("queue worker should boot");

    let root_topic = format!("harness-turn-{}", Uuid::new_v4());
    let subagent_topic = format!("harness-subagent-{}", Uuid::new_v4());
    let reactive_topic = format!("harness-reactive-{}", Uuid::new_v4());
    let function_id = format!("queue.replay.{}", Uuid::new_v4());
    let fires = Arc::new(AtomicUsize::new(0));
    register_counting_function(
        &harness_client,
        &function_id,
        fires.clone(),
        Arc::new(AtomicBool::new(false)),
    );
    for topic in [&root_topic, &subagent_topic, &reactive_topic] {
        register_subscriber_with_config(
            &harness_client,
            &function_id,
            topic,
            Some(json!({"type": "standard", "concurrency": 10})),
        );
    }
    wait_for_topics(
        &queue_client,
        &[&root_topic, &subagent_topic, &reactive_topic],
    )
    .await;

    boot.shutdown().await;
    queue_client.shutdown_async().await;

    let Some(restarted_queue_client) = engine::connect_fresh().await else {
        return;
    };
    let restarted = iii_queue::boot::start(restarted_queue_client.clone(), file_config(&dir))
        .await
        .expect("queue worker should restart");
    wait_for_topics(
        &restarted_queue_client,
        &[&root_topic, &subagent_topic, &reactive_topic],
    )
    .await;

    trigger(
        &restarted_queue_client,
        PUBLISH_FN_ID,
        json!({"queue": root_topic, "data": {"after_restart": true}}),
    )
    .await;
    wait_for_fires(&fires, 1).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(fires.load(Ordering::SeqCst), 1);

    restarted.shutdown().await;
    restarted_queue_client.shutdown_async().await;
    harness_client.shutdown_async().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
#[serial]
async fn publish_fails_after_queue_worker_disconnects_connect_or_skip() {
    let Some(queue_client) = engine::connect_fresh().await else {
        return;
    };
    let Some(caller_client) = engine::connect_fresh().await else {
        return;
    };
    let dir = temp_store_dir();
    let boot = iii_queue::boot::start(queue_client.clone(), file_config(&dir))
        .await
        .expect("queue worker should boot");

    boot.shutdown().await;
    queue_client.shutdown_async().await;

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let result = caller_client
            .trigger(TriggerRequest {
                function_id: PUBLISH_FN_ID.to_string(),
                payload: json!({"queue": "harness-turn", "data": {"turn_id": "t_1"}}),
                action: None,
                timeout_ms: Some(500),
            })
            .await;
        if result.is_err() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "publish continued reporting success after the queue worker disconnected"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    caller_client.shutdown_async().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
#[serial]
async fn delivery_preserves_originating_trace_lineage_connect_or_skip() {
    use iii_helpers::observability::opentelemetry::trace::FutureExt as OtelFutureExt;

    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let dir = temp_store_dir();
    let boot = iii_queue::boot::start(iii.clone(), file_config(&dir))
        .await
        .expect("queue worker should boot");

    let topic = format!("harness-trace-{}", Uuid::new_v4());
    let function_id = format!("queue.trace.{}", Uuid::new_v4());
    let observed = Arc::new(Mutex::new(None::<(Option<String>, Option<String>)>));
    iii.register_function(
        &function_id,
        RegisterFunction::new_async({
            let observed = observed.clone();
            move |_payload: Value| {
                let observed = observed.clone();
                async move {
                    *observed.lock().await = Some((
                        iii_helpers::observability::inject_traceparent(),
                        iii_helpers::observability::inject_baggage(),
                    ));
                    Ok::<Value, Error>(json!({"ok": true}))
                }
            }
        }),
    );
    register_subscriber(&iii, &function_id, &topic);
    wait_for_topics(&iii, &[&topic]).await;

    let source_traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let context =
        iii_helpers::observability::extract_context(Some(source_traceparent), Some("tenant=motia"));
    iii.trigger(TriggerRequest {
        function_id: PUBLISH_FN_ID.to_string(),
        payload: json!({"queue": topic, "data": {"turn_id": "t_1"}}),
        action: None,
        timeout_ms: Some(5_000),
    })
    .with_context(context)
    .await
    .expect("traced publish should succeed");

    let deadline = Instant::now() + Duration::from_secs(3);
    let (delivered_traceparent, delivered_baggage) = loop {
        if let Some(observed) = observed.lock().await.clone() {
            break observed;
        }
        assert!(
            Instant::now() < deadline,
            "subscriber did not observe the delivered trace context"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    let delivered_traceparent = delivered_traceparent.expect("delivery should be traced");
    assert_eq!(
        delivered_traceparent.split('-').nth(1),
        source_traceparent.split('-').nth(1),
        "queue delivery should remain in the originating trace"
    );
    assert!(
        delivered_baggage
            .as_deref()
            .is_some_and(|baggage| baggage.contains("tenant=motia")),
        "queue delivery should restore originating baggage"
    );

    boot.shutdown().await;
    iii.shutdown_async().await;
    let _ = std::fs::remove_dir_all(dir);
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
    // Unique queue name: the engine re-delivers previously registered
    // durable:subscriber triggers on boot, so a fixed name races against
    // leftover consumers from earlier local runs.
    let queue_name = format!("e2e-demo-{}", Uuid::new_v4());
    register_counting_function(&iii, &function_id, fires.clone(), fail.clone());
    register_subscriber(&iii, &function_id, &queue_name);

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
    let messages = wait_for_dlq(&iii, &queue_name).await;
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
async fn file_based_pending_message_survives_worker_restart_connect_or_skip() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };

    let dir = temp_store_dir();
    let boot = iii_queue::boot::start(iii.clone(), file_config(&dir))
        .await
        .expect("queue worker should boot");

    // Engine-parity restart scenario: the message must be enqueued onto a
    // SUBSCRIBED topic (fan-out routes it to the subscriber's internal
    // queue, which is what the file store persists). A publish with no
    // subscriber buffers on the bare topic and is never drained by a later
    // subscribe — same as the engine builtin. `concurrency: 0` pauses
    // consumption (engine semantics) so the message is still pending when
    // the worker shuts down.
    let queue_name = format!("e2e-restart-{}", Uuid::new_v4());
    let function_id = format!("queue.restart.{queue_name}");
    register_subscriber_with_config(
        &iii,
        &function_id,
        &queue_name,
        Some(json!({"concurrency": 0})),
    );
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    trigger(
        &iii,
        PUBLISH_FN_ID,
        json!({"queue": queue_name, "data": {"survives": true}}),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    boot.shutdown().await;
    iii.shutdown_async().await;

    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_queue::boot::start(iii.clone(), file_config(&dir))
        .await
        .expect("queue worker should reboot");

    let fires = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicBool::new(false));
    // Same function id as before the restart: the persisted job lives in
    // the internal queue `{topic}::{function_id}`.
    register_counting_function(&iii, &function_id, fires.clone(), fail);
    register_subscriber(&iii, &function_id, &queue_name);
    wait_for_fires(&fires, 1).await;

    boot.shutdown().await;
    iii.shutdown_async().await;
    let _ = std::fs::remove_dir_all(dir);
}
