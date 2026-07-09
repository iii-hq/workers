mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_queue::adapter::QueueAdapter;
use iii_queue::adapters::redis::RedisAdapter;
use iii_queue::trigger::IiiInvoker;
use iii_sdk::errors::Error;
use iii_sdk::RegisterFunction;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::time::Instant;
use uuid::Uuid;

use common::docker;
use common::engine;

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

/// Boots a `RedisAdapter` directly against a live engine connection and a
/// docker-backed Redis, bypassing `boot::start`/`QueueConfig` — the redis
/// adapter isn't wired into the config factory yet (a later task), so this
/// exercises the adapter the way that factory will: `from_config` + a
/// direct `subscribe`/`enqueue` against the trait, instead of going through
/// the `durable:subscriber` trigger type.
#[tokio::test]
#[serial]
async fn publish_delivers_unwrapped_payload_to_subscriber_connect_or_skip() {
    let Some(container) = docker::start_redis() else {
        return; // skip: docker not reachable
    };
    let Some(iii) = engine::connect_fresh().await else {
        return; // skip: engine not reachable
    };

    let invoker = Arc::new(IiiInvoker::new(iii.clone()));
    let adapter =
        RedisAdapter::from_config(Some(&json!({"redis_url": container.redis_url()})), invoker)
            .await
            .expect("redis adapter should connect");

    let fires = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
    let function_id = format!("queue.e2e.redis.{}", Uuid::new_v4());
    {
        let fires = fires.clone();
        let seen = seen.clone();
        iii.register_function(
            function_id.as_str(),
            RegisterFunction::new_async(move |payload: Value| {
                let fires = fires.clone();
                let seen = seen.clone();
                async move {
                    fires.fetch_add(1, Ordering::SeqCst);
                    seen.lock().await.push(payload);
                    Ok::<Value, Error>(json!({"ok": true}))
                }
            }),
        );
    }

    let topic = format!("e2e-redis-{}", Uuid::new_v4());
    adapter
        .subscribe(&topic, "sub-1", &function_id, None, None)
        .await;
    // Give the subscription task a beat to actually SUBSCRIBE on the Redis
    // connection before the first publish — pub/sub has no buffering for a
    // not-yet-subscribed consumer.
    tokio::time::sleep(Duration::from_millis(300)).await;

    adapter
        .enqueue(&topic, json!({"hello": "world"}), None, None)
        .await
        .unwrap();
    wait_for_fires(&fires, 1).await;

    let seen = seen.lock().await;
    // Parity with the builtin (and the pubsub worker's own e2e test): the
    // SDK stamps cross-worker invocation payloads with caller metadata
    // (e.g. `_caller_worker_id`), so assert the published field is present
    // rather than exact-equal on the whole object. The key assertion is
    // that this is the RAW data, not the `__trace` envelope
    // (`{"__trace": ..., "data": ...}`) — no `__trace`/`data` keys leak
    // through.
    assert_eq!(seen[0].get("hello"), Some(&json!("world")));
    assert!(seen[0].get("__trace").is_none());
    assert!(seen[0].get("data").is_none());
    drop(seen);

    adapter.unsubscribe(&topic, "sub-1").await;
    adapter.shutdown().await;
    iii.shutdown_async().await;
}

/// The redis adapter is pub/sub only: every DLQ-family method must fail
/// with the exact engine error string, never silently succeed.
#[tokio::test]
#[serial]
async fn dlq_operations_return_not_supported_connect_or_skip() {
    let Some(container) = docker::start_redis() else {
        return; // skip: docker not reachable
    };
    let Some(iii) = engine::connect_fresh().await else {
        return; // skip: engine not reachable
    };

    let invoker = Arc::new(IiiInvoker::new(iii.clone()));
    let adapter =
        RedisAdapter::from_config(Some(&json!({"redis_url": container.redis_url()})), invoker)
            .await
            .expect("redis adapter should connect");

    const NOT_SUPPORTED: &str = "RedisAdapter does not support DLQ operations (pub/sub only)";

    let topic = format!("e2e-redis-dlq-{}", Uuid::new_v4());

    let err = adapter.redrive_dlq(&topic).await.unwrap_err();
    assert_eq!(err.to_string(), NOT_SUPPORTED);

    let err = adapter
        .redrive_dlq_message(&topic, "some-id")
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), NOT_SUPPORTED);

    let err = adapter
        .discard_dlq_message(&topic, "some-id")
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), NOT_SUPPORTED);

    let err = adapter.dlq_count(&topic).await.unwrap_err();
    assert_eq!(err.to_string(), NOT_SUPPORTED);

    adapter.shutdown().await;
    iii.shutdown_async().await;
}
