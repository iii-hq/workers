//! `queue::on-message` against a live engine (connect-or-skip, like
//! `e2e_durability.rs`): the queue worker boots in-process in a private
//! namespace, and a second connection observes it next to an old-style
//! `durable:subscriber` binding. Skipped on an engine without observe.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_queue::config::QueueConfig;
use iii_queue::functions::{DLQ_MESSAGES_FN_ID, PUBLISH_FN_ID};
use iii_queue::on_message::ON_MESSAGE_FN;
use iii_queue::trigger::QueueTriggerHandler;
use iii_queue::TRIGGER_TYPE;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{IIIClient, InitOptions, RegisterFunction, Source, WorkerIdentityMode};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::time::Instant;
use uuid::Uuid;

use common::engine;

type Deliveries = Arc<Mutex<Vec<Value>>>;

async fn connect(namespace: &str, role: &str) -> Option<Arc<IIIClient>> {
    engine::connect_fresh_with_options(InitOptions {
        metadata: Some(WorkerMetadata {
            name: format!("{namespace}-{role}"),
            ..WorkerMetadata::default()
        }),
        namespace: Some(namespace.to_string()),
        identity: WorkerIdentityMode::Explicit,
        ..InitOptions::default()
    })
    .await
}

/// A target that records what it receives, failing every call when `fail`.
fn register_target(iii: &IIIClient, function_id: &str, fail: bool) -> Deliveries {
    let deliveries: Deliveries = Arc::default();
    let recorded = deliveries.clone();
    iii.register_function(
        function_id,
        RegisterFunction::new(move |payload: Value| {
            recorded.lock().unwrap().push(payload);
            if fail {
                Err(Error::Handler("e2e forced failure".to_string()))
            } else {
                Ok(json!({ "ok": true }))
            }
        }),
    );
    deliveries
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value, namespace: &str) -> Value {
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
    .expect("trigger should succeed")
}

async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(done(), "timed out waiting for {what}");
}

async fn wait_for_subscribers(handler: &QueueTriggerHandler, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if handler.registrations().await.len() == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("expected {expected} subscribers");
}

fn count(deliveries: &Deliveries) -> usize {
    deliveries.lock().unwrap().len()
}

/// Without the engine-stamped caller id, which names the worker that made
/// the call (the queue worker either way).
fn data(deliveries: &Deliveries) -> Vec<Value> {
    let mut out: Vec<Value> = deliveries
        .lock()
        .unwrap()
        .iter()
        .map(|payload| {
            let mut payload = payload.clone();
            if let Some(object) = payload.as_object_mut() {
                object.remove("_caller_worker_id");
            }
            payload
        })
        .collect();
    out.sort_by_key(|payload| payload["n"].as_i64());
    out
}

#[tokio::test]
#[serial]
async fn observer_and_trigger_on_one_topic_each_get_every_message_connect_or_skip() {
    let namespace = format!("queue-e2e-observe-{}", Uuid::new_v4());
    let Some(worker) = connect(&namespace, "queue").await else {
        return;
    };
    let Some(consumer) = connect(&namespace, "consumer").await else {
        worker.shutdown_async().await;
        return;
    };
    if consumer.engine_supports_observe() != Some(true) {
        eprintln!("[skip] engine without observe");
        consumer.shutdown_async().await;
        worker.shutdown_async().await;
        return;
    }
    let boot = iii_queue::boot::start(worker.clone(), QueueConfig::default())
        .await
        .expect("queue worker should boot");

    let topic = format!("orders-{}", Uuid::new_v4().simple());
    let via_trigger = register_target(&consumer, "e2e::orders::legacy", false);
    let via_observer = register_target(&consumer, "e2e::orders::observer", false);
    #[allow(deprecated)]
    consumer
        .register_trigger(RegisterTriggerInput::new(
            TRIGGER_TYPE.to_string(),
            "e2e::orders::legacy".to_string(),
            json!({ "queue": topic }),
        ))
        .expect("register durable subscriber trigger");
    let observer = consumer.observe(
        Source::function(ON_MESSAGE_FN).args(json!({ "topic": topic })),
        "e2e::orders::observer",
    );
    tokio::time::timeout(Duration::from_secs(5), observer.ready())
        .await
        .expect("observer ready in time")
        .expect("observer active");
    wait_for_subscribers(&boot.trigger_handler, 2).await;

    for n in 0..3 {
        call(
            &consumer,
            PUBLISH_FN_ID,
            json!({ "topic": topic, "data": { "n": n } }),
            &namespace,
        )
        .await;
    }
    wait_for("three deliveries each", || {
        count(&via_trigger) == 3 && count(&via_observer) == 3
    })
    .await;
    assert_eq!(data(&via_trigger), data(&via_observer), "same payloads");

    // Invoking the source directly is refused.
    let forged = consumer
        .trigger(
            TriggerRequest {
                function_id: ON_MESSAGE_FN.to_string(),
                payload: json!({ "n": 99 }),
                action: None,
                timeout_ms: Some(5_000),
            }
            .namespace(&namespace),
        )
        .await
        .expect_err("direct invocation must fail");
    assert!(forged.to_string().contains("FORBIDDEN"), "{forged}");

    // Disposing the observer stops its consumer; the trigger keeps going.
    observer.dispose().await.expect("dispose");
    wait_for_subscribers(&boot.trigger_handler, 1).await;
    call(
        &consumer,
        PUBLISH_FN_ID,
        json!({ "topic": topic, "data": { "n": 3 } }),
        &namespace,
    )
    .await;
    wait_for("the trigger's fourth delivery", || count(&via_trigger) == 4).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(count(&via_observer), 3, "no delivery after dispose");

    boot.shutdown().await;
    consumer.shutdown_async().await;
    worker.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn a_failing_observer_target_is_retried_then_dead_lettered_connect_or_skip() {
    let namespace = format!("queue-e2e-observe-dlq-{}", Uuid::new_v4());
    let Some(worker) = connect(&namespace, "queue").await else {
        return;
    };
    let Some(consumer) = connect(&namespace, "consumer").await else {
        worker.shutdown_async().await;
        return;
    };
    if consumer.engine_supports_observe() != Some(true) {
        eprintln!("[skip] engine without observe");
        consumer.shutdown_async().await;
        worker.shutdown_async().await;
        return;
    }
    let boot = iii_queue::boot::start(worker.clone(), QueueConfig::default())
        .await
        .expect("queue worker should boot");

    let topic = format!("failing-{}", Uuid::new_v4().simple());
    let attempts = register_target(&consumer, "e2e::orders::failing", true);
    let rejected = consumer.observe(
        Source::function(ON_MESSAGE_FN).args(json!({ "max_retries": 2 })),
        "e2e::orders::failing",
    );
    let error = tokio::time::timeout(Duration::from_secs(5), rejected.ready())
        .await
        .expect("rejected in time")
        .expect_err("no topic: rejected");
    assert!(error.to_string().contains("INVALID_ARGS"), "{error}");
    let observer = consumer.observe(
        Source::function(ON_MESSAGE_FN)
            .args(json!({ "topic": topic, "max_retries": 2, "backoff_ms": 5 })),
        "e2e::orders::failing",
    );
    tokio::time::timeout(Duration::from_secs(5), observer.ready())
        .await
        .expect("observer ready in time")
        .expect("observer active");

    call(
        &consumer,
        PUBLISH_FN_ID,
        json!({ "topic": topic, "data": { "n": 1 } }),
        &namespace,
    )
    .await;
    let mut dead = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while dead == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
        let messages = call(
            &consumer,
            DLQ_MESSAGES_FN_ID,
            json!({ "queue": topic, "limit": 10 }),
            &namespace,
        )
        .await;
        dead = messages.as_array().map(Vec::len).unwrap_or(0);
    }
    assert_eq!(dead, 1, "one dead-lettered message");
    assert_eq!(count(&attempts), 2, "two attempts with max_retries: 2");

    observer.dispose().await.expect("dispose");
    boot.shutdown().await;
    consumer.shutdown_async().await;
    worker.shutdown_async().await;
}
