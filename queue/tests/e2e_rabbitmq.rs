#![cfg(feature = "rabbitmq")]

mod common;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use async_trait::async_trait;
use iii_queue::adapter::{FunctionQueueConfig, QueueAdapter};
use iii_queue::adapters::rabbitmq::RabbitMQAdapter;
use iii_queue::subscriber_config::SubscriberQueueConfig;
use iii_queue::trigger::{IiiInvoker, Invoker};
use iii_sdk::errors::Error;
use iii_sdk::RegisterFunction;
use lapin::options::{ExchangeDeclareOptions, QueueBindOptions, QueueDeclareOptions};
use lapin::types::{AMQPValue, FieldTable};
use lapin::{Channel, Connection, ConnectionProperties, ExchangeKind};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::sync::{Mutex, Notify};
use tokio::time::Instant;
use uuid::Uuid;

use common::docker;
use common::engine;

/// Polls `pred` until it returns `true` or `timeout` elapses, then asserts
/// once more (for a useful panic message) -- same pattern as this crate's
/// `adapters::builtin` unit tests and `e2e_redis.rs`.
async fn wait_until<F, Fut>(mut pred: F, timeout: Duration)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(pred().await, "condition did not become true before timeout");
}

// -- Naming helpers mirroring `adapters::rabbitmq::naming::RabbitNames`
// (private to the crate) -- needed here only to pre-declare a subscriber's
// topology by hand for the priority test, see its comment below.
fn exchange_name(topic: &str) -> String {
    format!("iii.{topic}.exchange")
}
fn subscriber_queue_name(topic: &str, subscription_id: &str) -> String {
    format!("iii.{topic}.{subscription_id}.queue")
}
fn subscriber_dlq_name(topic: &str, subscription_id: &str) -> String {
    format!("iii.{topic}.{subscription_id}.dlq")
}

/// Declares the exact same topology `RabbitMQAdapter::subscribe` would
/// declare for `(topic, subscription_id)` -- fanout exchange, per-subscriber
/// DLQ, priority queue (`x-max-priority`), and the binding -- via a
/// raw `lapin` channel, so messages can be published into the queue BEFORE
/// any adapter-owned consumer exists. All declarations use identical
/// arguments to what `topology::TopologyManager` declares, so the later,
/// real `subscribe()` call's redeclare is a idempotent no-op (AMQP rejects a
/// redeclare with mismatched arguments).
async fn predeclare_priority_subscriber_queue(
    channel: &Channel,
    topic: &str,
    subscription_id: &str,
    max_priority: i32,
) {
    channel
        .exchange_declare(
            &exchange_name(topic),
            ExchangeKind::Fanout,
            ExchangeDeclareOptions {
                durable: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await
        .expect("declare fanout exchange");

    channel
        .queue_declare(
            &subscriber_dlq_name(topic, subscription_id),
            QueueDeclareOptions {
                durable: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await
        .expect("declare dlq");

    let mut args = FieldTable::default();
    args.insert("x-max-priority".into(), AMQPValue::LongInt(max_priority));
    channel
        .queue_declare(
            &subscriber_queue_name(topic, subscription_id),
            QueueDeclareOptions {
                durable: true,
                ..Default::default()
            },
            args,
        )
        .await
        .expect("declare priority queue");

    channel
        .queue_bind(
            &subscriber_queue_name(topic, subscription_id),
            &exchange_name(topic),
            "",
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await
        .expect("bind priority queue");
}

/// Never actually invoked -- used for the function-queue DLQ/redrive test,
/// which drives ack/nack directly through the `QueueAdapter` trait instead
/// of going through a live engine + registered function.
struct NoopInvoker;

#[async_trait]
impl Invoker for NoopInvoker {
    async fn call(&self, _function_id: &str, _payload: Value) -> Result<Option<Value>, String> {
        panic!("NoopInvoker::call should never be invoked in this test")
    }
}

#[derive(Default)]
struct RecordingInvoker {
    deliveries: Mutex<Vec<(String, Value, Option<Value>)>>,
}

#[async_trait]
impl Invoker for RecordingInvoker {
    async fn call(&self, _function_id: &str, _payload: Value) -> Result<Option<Value>, String> {
        Ok(None)
    }

    async fn call_delivery(
        &self,
        function_id: &str,
        payload: Value,
        metadata: Option<Value>,
    ) -> Result<Option<Value>, String> {
        self.deliveries
            .lock()
            .await
            .push((function_id.to_string(), payload, metadata));
        Ok(None)
    }
}

#[derive(Default)]
struct BlockingInvoker {
    started: Notify,
    dropped: AtomicBool,
    deliveries: Mutex<Vec<(Value, Option<Value>)>>,
}

struct DropFlag<'a>(&'a AtomicBool);

impl Drop for DropFlag<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl Invoker for BlockingInvoker {
    async fn call(&self, _function_id: &str, _payload: Value) -> Result<Option<Value>, String> {
        Ok(None)
    }

    async fn call_delivery(
        &self,
        _function_id: &str,
        payload: Value,
        metadata: Option<Value>,
    ) -> Result<Option<Value>, String> {
        if payload == json!("old") {
            let _drop_flag = DropFlag(&self.dropped);
            self.started.notify_one();
            std::future::pending::<()>().await;
            unreachable!();
        }
        self.deliveries.lock().await.push((payload, metadata));
        Ok(None)
    }
}

/// (a) Basic delivery: subscribe, enqueue, the registered function receives
/// the raw data.
#[tokio::test]
#[serial]
async fn basic_delivery_connect_or_skip() {
    let Some(container) = docker::start_rabbitmq().await else {
        return; // skip: docker not reachable
    };
    let Some(iii) = engine::connect_fresh().await else {
        return; // skip: engine not reachable
    };

    let invoker = Arc::new(IiiInvoker::new(iii.clone()));
    let adapter =
        RabbitMQAdapter::from_config(Some(&json!({"amqp_url": container.amqp_url()})), invoker)
            .await
            .expect("rabbitmq adapter should connect");

    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let function_id = format!("queue.e2e.rabbitmq.basic.{}", Uuid::new_v4());
    {
        let seen = seen.clone();
        iii.register_function(
            function_id.as_str(),
            RegisterFunction::new_async(move |payload: Value| {
                let seen = seen.clone();
                async move {
                    seen.lock().await.push(payload);
                    Ok::<Value, Error>(json!({"ok": true}))
                }
            }),
        );
    }

    let topic = format!("e2e-rmq-basic-{}", Uuid::new_v4());
    adapter
        .subscribe(&topic, "sub-1", &function_id, None, None, None)
        .await;
    // Give the consumer task a beat to actually attach before the first
    // publish.
    tokio::time::sleep(Duration::from_millis(300)).await;

    adapter
        .enqueue(&topic, json!({"hello": "world"}), None, None)
        .await;

    wait_until(
        || {
            let seen = seen.clone();
            async move { !seen.lock().await.is_empty() }
        },
        Duration::from_secs(10),
    )
    .await;

    let seen = seen.lock().await;
    assert_eq!(seen[0].get("hello"), Some(&json!("world")));
    drop(seen);

    adapter.unsubscribe(&topic, "sub-1").await;
    adapter.shutdown().await;
    iii.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn same_function_subscriptions_each_receive_their_metadata_connect_or_skip() {
    let Some(container) = docker::start_rabbitmq().await else {
        return; // skip: docker not reachable
    };

    let invoker = Arc::new(RecordingInvoker::default());
    let adapter = RabbitMQAdapter::from_config(
        Some(&json!({"amqp_url": container.amqp_url()})),
        invoker.clone(),
    )
    .await
    .expect("rabbitmq adapter should connect");
    let topic = format!("e2e-rmq-fanout-{}", Uuid::new_v4());

    adapter
        .subscribe(
            &topic,
            "sub-a",
            "same-function",
            Some(json!({"binding": "a"})),
            None,
            None,
        )
        .await;
    adapter
        .subscribe(
            &topic,
            "sub-b",
            "same-function",
            Some(json!({"binding": "b"})),
            None,
            None,
        )
        .await;
    adapter
        .enqueue(&topic, json!({"event": 1}), None, None)
        .await;

    wait_until(
        || {
            let invoker = invoker.clone();
            async move { invoker.deliveries.lock().await.len() == 2 }
        },
        Duration::from_secs(10),
    )
    .await;

    let deliveries = invoker.deliveries.lock().await;
    assert!(deliveries
        .iter()
        .all(|(function_id, payload, _)| function_id == "same-function"
            && payload == &json!({"event": 1})));
    assert!(deliveries
        .iter()
        .any(|(_, _, metadata)| metadata == &Some(json!({"binding": "a"}))));
    assert!(deliveries
        .iter()
        .any(|(_, _, metadata)| metadata == &Some(json!({"binding": "b"}))));
    drop(deliveries);

    // A true unsubscribe deletes its durable queue. If it only cancelled the
    // consumer, event 2 would remain in that queue and be delivered when the
    // same subscription id is armed again.
    adapter.unsubscribe(&topic, "sub-a").await;
    adapter
        .enqueue(&topic, json!({"event": 2}), None, None)
        .await;
    wait_until(
        || {
            let invoker = invoker.clone();
            async move { invoker.deliveries.lock().await.len() == 3 }
        },
        Duration::from_secs(10),
    )
    .await;
    adapter
        .subscribe(
            &topic,
            "sub-a",
            "same-function",
            Some(json!({"binding": "a-rearmed"})),
            None,
            None,
        )
        .await;
    adapter
        .enqueue(&topic, json!({"event": 3}), None, None)
        .await;
    wait_until(
        || {
            let invoker = invoker.clone();
            async move { invoker.deliveries.lock().await.len() == 5 }
        },
        Duration::from_secs(10),
    )
    .await;
    let deliveries = invoker.deliveries.lock().await;
    assert_eq!(
        deliveries
            .iter()
            .filter(|(_, payload, _)| payload == &json!({"event": 2}))
            .count(),
        1
    );
    assert!(deliveries.iter().any(|(_, payload, metadata)| {
        payload == &json!({"event": 3}) && metadata == &Some(json!({"binding": "a-rearmed"}))
    }));
    drop(deliveries);

    adapter.unsubscribe(&topic, "sub-a").await;
    adapter.unsubscribe(&topic, "sub-b").await;
    adapter.shutdown().await;
}

#[tokio::test]
#[serial]
async fn unsubscribe_joins_inflight_delivery_before_queue_delete_and_rearm_connect_or_skip() {
    let Some(container) = docker::start_rabbitmq().await else {
        return;
    };

    let invoker = Arc::new(BlockingInvoker::default());
    let adapter = RabbitMQAdapter::from_config(
        Some(&json!({"amqp_url": container.amqp_url()})),
        invoker.clone(),
    )
    .await
    .expect("rabbitmq adapter should connect");
    let topic = format!("e2e-rmq-rearm-{}", Uuid::new_v4());
    let queue_config = Some(SubscriberQueueConfig {
        queue_mode: Some("standard".to_string()),
        concurrency: Some(2),
        ..Default::default()
    });

    adapter
        .subscribe(
            &topic,
            "sub-1",
            "backend",
            Some(json!({"binding": "old"})),
            None,
            queue_config.clone(),
        )
        .await;
    adapter.enqueue(&topic, json!("old"), None, None).await;
    tokio::time::timeout(Duration::from_secs(10), invoker.started.notified())
        .await
        .expect("old delivery should start");

    tokio::time::timeout(
        Duration::from_secs(10),
        adapter.unsubscribe(&topic, "sub-1"),
    )
    .await
    .expect("unsubscribe should cancel and join the old delivery");
    assert!(invoker.dropped.load(Ordering::SeqCst));

    adapter
        .subscribe(
            &topic,
            "sub-1",
            "backend",
            Some(json!({"binding": "new"})),
            None,
            queue_config,
        )
        .await;
    adapter.enqueue(&topic, json!("new"), None, None).await;
    wait_until(
        || {
            let invoker = invoker.clone();
            async move { invoker.deliveries.lock().await.len() == 1 }
        },
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(
        invoker.deliveries.lock().await.as_slice(),
        [(json!("new"), Some(json!({"binding": "new"})))]
    );

    adapter.unsubscribe(&topic, "sub-1").await;
    adapter.shutdown().await;
}

/// (b) Failing handler retries per `max_attempts`, lands in the DLQ, and
/// `redrive_dlq` redelivers it so it can succeed.
///
/// Exercised through the function-queue trait methods
/// (`setup_function_queue`/`publish_to_function_queue`/
/// `consume_function_queue`/`ack_function_queue`/`nack_function_queue`)
/// rather than through `subscribe`'s topic-fanout path. Discovered while
/// writing this test: `RabbitMQAdapter::resolve_dlq_name` (and therefore
/// `dlq_count`/`redrive_dlq`/`redrive_dlq_message`/`discard_dlq_message`)
/// resolves a bare (non-`__fn_queue::`) topic to `RabbitNames::dlq()`
/// (`iii.{topic}.dlq`) -- a queue name that `subscribe`'s
/// `setup_subscriber_queue` never actually declares (it declares
/// `RabbitNames::subscriber_dlq(subscription_id)`, i.e.
/// `iii.{topic}.{subscription_id}.dlq`, a *different* name). That mismatch is
/// inherited verbatim from the engine
/// (`engine/src/workers/queue/adapters/rabbitmq/adapter.rs`) -- the engine's
/// own `rabbitmq_queue_integration.rs` test suite never calls
/// `dlq_count`/`redrive_dlq` against a bare subscribed topic either (only
/// against `__fn_queue::`-prefixed function-queue names, which resolve
/// correctly). A passive `queue_declare` against a queue that doesn't exist
/// is a channel-level AMQP error, which would poison this adapter's shared
/// channel for every other operation -- so this test deliberately exercises
/// the function-queue path, which resolves correctly end-to-end, instead of
/// tripping that pre-existing engine gap.
#[tokio::test]
#[serial]
async fn function_queue_retry_then_dlq_then_redrive_connect_or_skip() {
    let Some(container) = docker::start_rabbitmq().await else {
        return; // skip: docker not reachable
    };

    let invoker: Arc<dyn Invoker> = Arc::new(NoopInvoker);
    let adapter =
        RabbitMQAdapter::from_config(Some(&json!({"amqp_url": container.amqp_url()})), invoker)
            .await
            .expect("rabbitmq adapter should connect");

    let queue_name = format!("e2e-rmq-fnq-{}", Uuid::new_v4());
    let dlq_topic = format!("__fn_queue::{queue_name}");

    adapter
        .setup_function_queue(
            &queue_name,
            &FunctionQueueConfig {
                max_retries: 1,
                concurrency: 1,
                backoff_ms: 200,
                ..FunctionQueueConfig::default()
            },
        )
        .await
        .expect("setup_function_queue should succeed");

    adapter
        .publish_to_function_queue(
            &queue_name,
            "target-fn",
            json!({"n": 1}),
            "msg-1",
            1,
            200,
            None,
            None,
            None,
        )
        .await
        .expect("publish_to_function_queue should succeed");

    let mut rx = adapter
        .consume_function_queue(&queue_name, 10)
        .await
        .expect("consume_function_queue should succeed");

    let msg1 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("first delivery should arrive")
        .expect("channel should stay open");
    assert_eq!(msg1.attempt, 0);
    adapter
        .nack_function_queue(&queue_name, msg1.delivery_id, msg1.attempt, 1)
        .await
        .expect("nack (retry) should succeed");

    let msg2 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("retried delivery should arrive")
        .expect("channel should stay open");
    assert_eq!(msg2.attempt, 1, "attempt should be incremented on retry");
    adapter
        .nack_function_queue(&queue_name, msg2.delivery_id, msg2.attempt, 1)
        .await
        .expect("nack (exhausted) should succeed");

    wait_until(
        || {
            let adapter = &adapter;
            let dlq_topic = dlq_topic.clone();
            async move { adapter.dlq_count(&dlq_topic).await.unwrap_or(0) >= 1 }
        },
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(adapter.dlq_count(&dlq_topic).await.unwrap(), 1);

    let redriven = adapter
        .redrive_dlq(&dlq_topic)
        .await
        .expect("redrive_dlq should succeed");
    assert_eq!(redriven, 1);
    assert_eq!(adapter.dlq_count(&dlq_topic).await.unwrap(), 0);

    let msg3 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("redriven delivery should arrive")
        .expect("channel should stay open");
    assert_eq!(msg3.attempt, 0, "redrive resets the attempt counter");
    adapter
        .ack_function_queue(&queue_name, msg3.delivery_id)
        .await
        .expect("ack should succeed");

    adapter.shutdown().await;
}

/// (c) Priority ordering: 3 messages, published with `priority_field`
/// values 1, 9, 5 -- in that order -- onto a subscriber queue declared with
/// `max_priority: 10`, all BEFORE any consumer attaches (the topology is
/// pre-declared by hand via a raw `lapin` channel so the fanout-published
/// messages have somewhere to land). `subscribe()` is only called after all
/// three are confirmed-published, so the very first delivery already
/// reflects priority order: 9, 5, 1.
#[tokio::test]
#[serial]
async fn priority_ordering_connect_or_skip() {
    let Some(container) = docker::start_rabbitmq().await else {
        return; // skip: docker not reachable
    };
    let Some(iii) = engine::connect_fresh().await else {
        return; // skip: engine not reachable
    };

    let invoker = Arc::new(IiiInvoker::new(iii.clone()));
    let adapter = RabbitMQAdapter::from_config(
        Some(&json!({"amqp_url": container.amqp_url(), "priority_field": "priority"})),
        invoker,
    )
    .await
    .expect("rabbitmq adapter should connect");

    let function_id = format!("queue.e2e.rabbitmq.priority.{}", Uuid::new_v4());
    let order = Arc::new(Mutex::new(Vec::<u64>::new()));
    {
        let order = order.clone();
        iii.register_function(
            function_id.as_str(),
            RegisterFunction::new_async(move |payload: Value| {
                let order = order.clone();
                async move {
                    let marker = payload.get("marker").and_then(|v| v.as_u64()).unwrap_or(0);
                    order.lock().await.push(marker);
                    Ok::<Value, Error>(json!({"ok": true}))
                }
            }),
        );
    }

    let topic = format!("e2e-rmq-priority-{}", Uuid::new_v4());
    let sub_id = "sub-priority-1";

    let raw_connection =
        Connection::connect(&container.amqp_url(), ConnectionProperties::default())
            .await
            .expect("raw amqp connection");
    let raw_channel = raw_connection
        .create_channel()
        .await
        .expect("raw amqp channel");
    predeclare_priority_subscriber_queue(&raw_channel, &topic, sub_id, 10).await;

    for p in [1u64, 9, 5] {
        adapter
            .enqueue(&topic, json!({"marker": p, "priority": p}), None, None)
            .await;
    }

    // No consumer exists yet at this point -- all three publishes above were
    // already broker-confirmed (`Publisher::publish` awaits the publish
    // confirm) before `subscribe` is called below.
    adapter
        .subscribe(
            &topic,
            sub_id,
            &function_id,
            None,
            None,
            Some(SubscriberQueueConfig {
                max_priority: Some(10),
                concurrency: Some(1),
                ..Default::default()
            }),
        )
        .await;

    wait_until(
        || {
            let order = order.clone();
            async move { order.lock().await.len() >= 3 }
        },
        Duration::from_secs(10),
    )
    .await;

    assert_eq!(
        *order.lock().await,
        vec![9, 5, 1],
        "priority queue should drain highest-priority-first"
    );

    adapter.unsubscribe(&topic, sub_id).await;
    adapter.shutdown().await;
    let _ = raw_connection.close(200, "test done").await;
    iii.shutdown_async().await;
}

/// (d) Fifo mode: 10 messages published in order are delivered in the same
/// order. Handler sleeps a small jittered delay so a broken fifo consumer
/// (e.g. one that spawns handlers concurrently instead of processing them
/// one at a time) would visibly scramble the order.
#[tokio::test]
#[serial]
async fn fifo_mode_preserves_order_connect_or_skip() {
    let Some(container) = docker::start_rabbitmq().await else {
        return; // skip: docker not reachable
    };
    let Some(iii) = engine::connect_fresh().await else {
        return; // skip: engine not reachable
    };

    let invoker = Arc::new(IiiInvoker::new(iii.clone()));
    let adapter = RabbitMQAdapter::from_config(
        Some(&json!({"amqp_url": container.amqp_url(), "queue_mode": "fifo"})),
        invoker,
    )
    .await
    .expect("rabbitmq adapter should connect");

    let function_id = format!("queue.e2e.rabbitmq.fifo.{}", Uuid::new_v4());
    let order = Arc::new(Mutex::new(Vec::<u64>::new()));
    {
        let order = order.clone();
        iii.register_function(
            function_id.as_str(),
            RegisterFunction::new_async(move |payload: Value| {
                let order = order.clone();
                async move {
                    let n = payload.get("n").and_then(|v| v.as_u64()).unwrap_or(0);
                    tokio::time::sleep(Duration::from_millis((n % 3) * 5)).await;
                    order.lock().await.push(n);
                    Ok::<Value, Error>(json!({"ok": true}))
                }
            }),
        );
    }

    let topic = format!("e2e-rmq-fifo-{}", Uuid::new_v4());
    adapter
        .subscribe(
            &topic,
            "sub-fifo-1",
            &function_id,
            None,
            None,
            Some(SubscriberQueueConfig {
                queue_mode: Some("fifo".to_string()),
                concurrency: Some(1),
                ..Default::default()
            }),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    for n in 0..10u64 {
        adapter.enqueue(&topic, json!({"n": n}), None, None).await;
    }

    wait_until(
        || {
            let order = order.clone();
            async move { order.lock().await.len() >= 10 }
        },
        Duration::from_secs(15),
    )
    .await;

    let expected: Vec<u64> = (0..10).collect();
    assert_eq!(
        *order.lock().await,
        expected,
        "fifo mode should preserve publish order"
    );

    adapter.unsubscribe(&topic, "sub-fifo-1").await;
    adapter.shutdown().await;
    iii.shutdown_async().await;
}
