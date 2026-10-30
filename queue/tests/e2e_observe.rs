//! `queue::on-message` against a live engine (connect-or-skip, like
//! `e2e_durability.rs`): the queue worker boots in-process in a private
//! namespace, and a second connection subscribes by observing it. Skipped on
//! an engine without observe.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_queue::config::QueueConfig;
use iii_queue::functions::{DLQ_MESSAGES_FN_ID, PUBLISH_FN_ID};
use iii_queue::on_message::ON_MESSAGE_FN;
use iii_queue::subscribers::Subscribers;
use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    CallRequest, IIIClient, InitOptions, ObserverHandle, ObserverOptions, RegisterFunction, Source,
    WorkerIdentityMode,
};
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
    iii.call(
        CallRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(5_000),
        }
        .namespace(namespace),
    )
    .await
    .expect("call should succeed")
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

/// Wait until `topic` has `expected` subscribers. The engine observes
/// `queue::on-message` too (its telemetry topics, in every namespace the
/// queue worker runs in), so only the subscribers of `topic` count.
async fn wait_for_subscribers(subscribers: &Subscribers, topic: &str, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let on_topic = subscribers
            .registrations()
            .await
            .iter()
            .filter(|registration| registration.spec.queue == topic)
            .count();
        if on_topic == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("expected {expected} subscribers of {topic}");
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
async fn each_observer_of_a_topic_gets_every_message_its_filters_admit_connect_or_skip() {
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
    let via_first = register_target(&consumer, "e2e::orders::first", false);
    let via_second = register_target(&consumer, "e2e::orders::second", false);
    let via_filtered = register_target(&consumer, "e2e::orders::filtered", false);
    let first = consumer.observe(
        Source::function(ON_MESSAGE_FN).args(json!({ "topic": topic })),
        "e2e::orders::first",
    );
    let second = consumer.observe(
        Source::function(ON_MESSAGE_FN).args(json!({ "topic": topic })),
        "e2e::orders::second",
    );
    // Filters read the message as input; the messages they reject are acked.
    let filtered = consumer.observe(
        Source::function(ON_MESSAGE_FN)
            .args(json!({ "topic": topic }))
            .filters(json!({ "input.n": { "$gte": 2 } })),
        "e2e::orders::filtered",
    );
    for observer in [&first, &second, &filtered] {
        tokio::time::timeout(Duration::from_secs(5), observer.ready())
            .await
            .expect("observer ready in time")
            .expect("observer active");
    }
    wait_for_subscribers(&boot.subscribers, &topic, 3).await;

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
        count(&via_first) == 3 && count(&via_second) == 3
    })
    .await;
    assert_eq!(data(&via_first), data(&via_second), "same payloads");
    wait_for("the filtered delivery", || count(&via_filtered) == 1).await;
    assert_eq!(data(&via_filtered), vec![json!({ "n": 2 })]);
    let dead = call(
        &consumer,
        DLQ_MESSAGES_FN_ID,
        json!({ "queue": topic, "limit": 10 }),
        &namespace,
    )
    .await;
    assert_eq!(
        dead.as_array().map(Vec::len),
        Some(0),
        "filter misses are acked"
    );

    // The function is private: a call from another worker is refused.
    let forged = consumer
        .call(
            CallRequest {
                function_id: ON_MESSAGE_FN.to_string(),
                payload: json!({ "n": 99 }),
                action: None,
                timeout_ms: Some(5_000),
            }
            .namespace(&namespace),
        )
        .await
        .expect_err("a call from another worker must fail");
    assert!(forged.to_string().contains("FORBIDDEN"), "{forged}");

    // Disposing an observer stops its consumer; the others keep going.
    first.dispose().await.expect("dispose");
    wait_for_subscribers(&boot.subscribers, &topic, 2).await;
    call(
        &consumer,
        PUBLISH_FN_ID,
        json!({ "topic": topic, "data": { "n": 3 } }),
        &namespace,
    )
    .await;
    wait_for("the second observer's fourth delivery", || {
        count(&via_second) == 4
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(count(&via_first), 3, "no delivery after dispose");

    second.dispose().await.expect("dispose");
    filtered.dispose().await.expect("dispose");
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

/// Publish `{ "n": n }` to `topic`.
async fn publish(iii: &IIIClient, topic: &str, n: i64, namespace: &str) {
    call(
        iii,
        PUBLISH_FN_ID,
        json!({ "topic": topic, "data": { "n": n } }),
        namespace,
    )
    .await;
}

/// The messages in `topic`'s DLQ.
async fn dead_letters(iii: &IIIClient, topic: &str, namespace: &str) -> usize {
    call(
        iii,
        DLQ_MESSAGES_FN_ID,
        json!({ "queue": topic, "limit": 100 }),
        namespace,
    )
    .await
    .as_array()
    .map(Vec::len)
    .unwrap_or_default()
}

/// A member of the consumer group `billing` on `topic`, observing under the
/// stable id `id`.
fn join_group(iii: &IIIClient, topic: &str, id: &str, target: &str) -> ObserverHandle {
    iii.observe_with(
        Source::function(ON_MESSAGE_FN).args(json!({
            "topic": topic,
            "group": "billing",
            "max_retries": 10,
            "backoff_ms": 20,
        })),
        target,
        ObserverOptions::default().id(id),
    )
}

async fn active(observer: &ObserverHandle) {
    tokio::time::timeout(Duration::from_secs(5), observer.ready())
        .await
        .expect("observer ready in time")
        .expect("observer active");
}

fn messages(range: std::ops::RangeInclusive<i64>) -> Vec<Value> {
    range.map(|n| json!({ "n": n })).collect()
}

/// Two members of one consumer group, and the one the group delivers into
/// is killed: the engine withdraws its observer from the queue worker when
/// its connection closes, the group's subscription moves to the member that
/// stays, and every message published after the kill reaches that member
/// once, with nothing dead-lettered. While the killed member is away its
/// stable id is taken for any other worker; back on a new connection under
/// the same worker name, its observe under that id replaces the suspended
/// observer and it rejoins the group, which still handles each message once.
#[tokio::test]
#[serial]
async fn a_killed_consumer_group_member_leaves_every_later_message_to_the_live_member_connect_or_skip(
) {
    let namespace = format!("queue-e2e-group-{}", Uuid::new_v4());
    let Some(worker) = connect(&namespace, "queue").await else {
        return;
    };
    let Some(member_a) = connect(&namespace, "member-a").await else {
        worker.shutdown_async().await;
        return;
    };
    let Some(member_b) = connect(&namespace, "member-b").await else {
        member_a.shutdown_async().await;
        worker.shutdown_async().await;
        return;
    };
    if member_b.engine_supports_observe() != Some(true) {
        eprintln!("[skip] engine without observe");
        member_b.shutdown_async().await;
        member_a.shutdown_async().await;
        worker.shutdown_async().await;
        return;
    }
    let boot = iii_queue::boot::start(worker.clone(), QueueConfig::default())
        .await
        .expect("queue worker should boot");

    let topic = format!("billing-{}", Uuid::new_v4().simple());
    let via_a = register_target(&member_a, "e2e::billing::a", false);
    let via_b = register_target(&member_b, "e2e::billing::b", false);
    // Member A joins first, so the group's one subscription delivers into it.
    let a = join_group(&member_a, &topic, "billing-member-a", "e2e::billing::a");
    active(&a).await;
    wait_for_subscribers(&boot.subscribers, &topic, 1).await;
    let b = join_group(&member_b, &topic, "billing-member-b", "e2e::billing::b");
    active(&b).await;
    wait_for_subscribers(&boot.subscribers, &topic, 2).await;

    publish(&member_b, &topic, 0, &namespace).await;
    wait_for("the group's first delivery", || count(&via_a) == 1).await;
    assert_eq!(count(&via_b), 0, "the group delivers into one member");

    // Member A dies: its connection closes and its observer is never
    // disposed (suspended for the engine's grace period).
    member_a.shutdown_async().await;
    for n in 1..=5 {
        publish(&member_b, &topic, n, &namespace).await;
    }
    wait_for("the live member's deliveries", || count(&via_b) == 5).await;
    wait_for_subscribers(&boot.subscribers, &topic, 1).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(data(&via_b), messages(1..=5), "each message once");
    assert_eq!(count(&via_a), 1, "nothing for the member that is away");
    assert_eq!(dead_letters(&member_b, &topic, &namespace).await, 0);

    // Its id stays taken for any other worker until the grace period ends.
    let squatter = join_group(&member_b, &topic, "billing-member-a", "e2e::billing::b");
    let conflict = tokio::time::timeout(Duration::from_secs(5), squatter.ready())
        .await
        .expect("answered in time")
        .expect_err("another worker cannot take a suspended observer's id");
    assert!(
        conflict.to_string().contains("OBSERVER_ID_CONFLICT"),
        "{conflict}"
    );

    // Member A restarts: a new connection under the same worker name, in the
    // same namespace, observes under the same id and replaces the suspended
    // observer; the queue worker files it with the group again.
    let member_a = connect(&namespace, "member-a")
        .await
        .expect("member A reconnects");
    let via_a_again = register_target(&member_a, "e2e::billing::a", false);
    let a_again = join_group(&member_a, &topic, "billing-member-a", "e2e::billing::a");
    active(&a_again).await;
    wait_for_subscribers(&boot.subscribers, &topic, 2).await;

    for n in 6..=8 {
        publish(&member_b, &topic, n, &namespace).await;
    }
    wait_for("the group's later deliveries", || {
        count(&via_a_again) + count(&via_b) == 8
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut later: Vec<Value> = data(&via_a_again)
        .into_iter()
        .chain(data(&via_b).into_iter().skip(5))
        .collect();
    later.sort_by_key(|payload| payload["n"].as_i64());
    assert_eq!(
        later,
        messages(6..=8),
        "the group handles each message once"
    );
    assert_eq!(dead_letters(&member_b, &topic, &namespace).await, 0);

    a_again.dispose().await.expect("dispose");
    b.dispose().await.expect("dispose");
    boot.shutdown().await;
    member_a.shutdown_async().await;
    member_b.shutdown_async().await;
    worker.shutdown_async().await;
}

/// A consumer outside a group, observing under a stable id, dies with
/// messages it has not acked (each failed once and waits for its retry). Its
/// worker restarts on a new connection under the same worker name, in the
/// same namespace, and observes under the same id: the new observer replaces
/// the one left behind, reattaches to the same durable queue, and receives
/// that backlog once.
#[tokio::test]
#[serial]
async fn a_restarted_consumer_observing_under_its_stable_id_receives_its_backlog_connect_or_skip() {
    let namespace = format!("queue-e2e-restart-{}", Uuid::new_v4());
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
    let subscribe = |iii: &IIIClient| {
        iii.observe_with(
            Source::function(ON_MESSAGE_FN).args(json!({
                "topic": topic,
                "max_retries": 5,
                "backoff_ms": 1_500,
            })),
            "e2e::orders::stable",
            ObserverOptions::default().id("orders-stable"),
        )
    };
    let failed = register_target(&consumer, "e2e::orders::stable", true);
    active(&subscribe(&consumer)).await;
    wait_for_subscribers(&boot.subscribers, &topic, 1).await;
    for n in 1..=3 {
        publish(&consumer, &topic, n, &namespace).await;
    }
    wait_for("each message's first, failed delivery", || {
        count(&failed) == 3
    })
    .await;

    consumer.shutdown_async().await;
    wait_for_subscribers(&boot.subscribers, &topic, 0).await;

    let consumer = connect(&namespace, "consumer")
        .await
        .expect("the consumer reconnects");
    let delivered = register_target(&consumer, "e2e::orders::stable", false);
    let observer = subscribe(&consumer);
    active(&observer).await;
    wait_for_subscribers(&boot.subscribers, &topic, 1).await;
    wait_for("the backlog's retries", || count(&delivered) == 3).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(data(&delivered), messages(1..=3), "each message once");
    assert_eq!(dead_letters(&consumer, &topic, &namespace).await, 0);

    observer.dispose().await.expect("dispose");
    boot.shutdown().await;
    consumer.shutdown_async().await;
    worker.shutdown_async().await;
}
