//! `queue::on-message`: the `durable:subscriber` trigger type as an
//! observable function (iii 0.25 observe).
//!
//! The function is owner-delivered. Its observers' args are a
//! `durable:subscriber` trigger's config ([`SubscriberSpec`]:
//! `{ topic, max_retries?, backoff_ms?, condition_function_id?, queue_config? }`).
//! [`OnMessageObservers`] is its [`ObserveHandler`]: each observer added
//! becomes one more subscriber of the [`QueueTriggerHandler`] that serves the
//! trigger type, validated by the same code, with a
//! [`SubscriberTarget::Observer`] target; a removed observer (disposed, or its
//! worker gone past the engine's grace period) is unregistered exactly like
//! an unregistered trigger. The adapters' consumer loops hand each message to
//! the observer with `obs.call`, and the target's result acks the message, or
//! retries it and then dead-letters it, as a trigger target's result does.
//!
//! The function itself refuses invocations (`FORBIDDEN`): messages reach
//! observers through the consumers only.
//!
//! Old consumers keep binding the `durable:subscriber` trigger type, which
//! this worker still registers on every engine. The function deliberately
//! declares no `legacy_trigger_type`: this worker is that trigger type's
//! provider, and moving old consumers onto the engine's legacy shim belongs
//! with retiring the trigger type. On an engine without observe the SDK
//! registers this function as a plain function and its hooks never run.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::observe_codes;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::trigger::{
    observer_key, ObserverSink, QueueTriggerHandler, RegisterError, RegisteredSubscriber,
    SubscriberSpec, SubscriberTarget, FUNCTION_QUEUE_INVOCATION_TIMEOUT_MS,
};

/// The observable function a queue topic's messages are delivered through.
pub const ON_MESSAGE_FN: &str = "queue::on-message";

pub const ON_MESSAGE_DESCRIPTION: &str =
    "One invocation per message published to a durable queue topic. Observe it with args \
     { topic, max_retries?, backoff_ms?, condition_function_id?, queue_config? } (the \
     durable:subscriber trigger config); every observer receives every message of its topic. \
     Your target receives the published data; returning acks the message, an error retries it \
     with backoff and then moves it to the topic's DLQ. Only the queue worker delivers its \
     messages: invoking it directly is refused.";

/// Registers `queue::on-message` with an [`OnMessageObservers`] over
/// `subscribers` as its hooks.
pub fn register(iii: &IIIClient, subscribers: QueueTriggerHandler) -> FunctionRef {
    iii.register_function(
        ON_MESSAGE_FN,
        RegisterFunction::new(refuse_invocation)
            .description(ON_MESSAGE_DESCRIPTION)
            .observe(
                Observe::owner()
                    .args_format_value(args_format())
                    .handler(OnMessageObservers::new(subscribers)),
            ),
    )
}

/// What a `queue::on-message` target receives: the data published to the
/// topic, unchanged. Published data is not always an object, so this keeps a
/// lossless `Value` while publishing a real schema (the registry publish gate
/// rejects `AnyValue`).
#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct QueueMessage(pub Value);

impl JsonSchema for QueueMessage {
    fn schema_name() -> String {
        "QueueMessage".to_string()
    }

    fn json_schema(_: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, Metadata, SchemaObject};
        SchemaObject {
            instance_type: Some(
                vec![
                    InstanceType::Null,
                    InstanceType::Boolean,
                    InstanceType::Number,
                    InstanceType::String,
                    InstanceType::Array,
                    InstanceType::Object,
                ]
                .into(),
            ),
            metadata: Some(Box::new(Metadata {
                description: Some("The data published to the topic, as published.".to_string()),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

/// The function's handler. Messages are delivered by the consumers with
/// `obs.call`, never by invoking the function: a caller allowed to invoke it
/// could otherwise inject messages that were never published.
fn refuse_invocation(_message: QueueMessage) -> Result<(), Error> {
    Err(Error::Remote {
        code: observe_codes::FORBIDDEN.to_string(),
        message: format!(
            "{ON_MESSAGE_FN} is delivered by the queue worker: observe it \
             (iii.observe({{ function_id: '{ON_MESSAGE_FN}', args: {{ topic }} }}, target)) \
             and publish with iii::durable::publish"
        ),
        stacktrace: None,
    })
}

/// The observers' `args_format`: [`SubscriberSpec`]'s schema plus the
/// aliases its serde config accepts (`topic` for `queue`, `maxRetries`,
/// `backoffDelayMs`), with one of `queue` / `topic` required. The engine then
/// admits exactly the configs the trigger type does (`INVALID_ARGS`
/// otherwise); [`parse_args`] and the shared registration path do the rest.
pub fn args_format() -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(SubscriberSpec))
        .expect("a JSON Schema serializes");
    if let Some(object) = schema.as_object_mut() {
        if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
            for (field, alias) in [
                ("queue", "topic"),
                ("max_retries", "maxRetries"),
                ("backoff_ms", "backoffDelayMs"),
            ] {
                if let Some(field_schema) = properties.get(field).cloned() {
                    properties.insert(alias.to_string(), field_schema);
                }
            }
        }
        object.remove("required");
        object.insert(
            "anyOf".to_string(),
            json!([{ "required": ["queue"] }, { "required": ["topic"] }]),
        );
    }
    schema
}

/// An observer's args, parsed as the trigger type parses its config.
pub fn parse_args(args: &Value) -> Result<SubscriberSpec, ObserveRejection> {
    serde_json::from_value(args.clone()).map_err(|e| {
        ObserveRejection::new(
            observe_codes::INVALID_ARGS,
            format!("invalid {ON_MESSAGE_FN} args: {e}"),
        )
    })
}

/// What a finished `obs.call` means for the message: the target's result
/// acks it; the observer's filters skipping it acks it without a delivery,
/// as a `condition_function_id` returning `false` does; any other error
/// (the target's own, an unreachable target, a suspended observer:
/// `OBSERVER_NOT_ACTIVE`) fails it, so it is retried and then dead-lettered.
pub fn call_outcome(result: Result<Value, Error>) -> Result<Option<Value>, String> {
    match result {
        Ok(output) => Ok(Some(output)),
        Err(Error::Remote { code, .. }) if code == observe_codes::FILTERS_NOT_MATCHED => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[async_trait]
impl ObserverSink for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn target(&self) -> &str {
        Observer::target(self)
    }

    async fn call(&self, payload: Value) -> Result<Option<Value>, String> {
        call_outcome(
            self.call_with_timeout::<Value>(
                payload,
                Duration::from_millis(FUNCTION_QUEUE_INVOCATION_TIMEOUT_MS),
            )
            .await,
        )
    }
}

/// The hooks of `queue::on-message`: files each observer as a subscriber.
#[derive(Clone)]
pub struct OnMessageObservers {
    subscribers: QueueTriggerHandler,
}

impl OnMessageObservers {
    pub fn new(subscribers: QueueTriggerHandler) -> Self {
        Self { subscribers }
    }

    /// Registers the observer behind `sink` as a subscriber of `spec`'s
    /// topic. Its consumer starts here, before the SDK acks the observer, so
    /// no message published after `ready()` resolves is missed.
    pub async fn add(
        &self,
        sink: Arc<dyn ObserverSink>,
        spec: SubscriberSpec,
    ) -> Result<(), ObserveRejection> {
        self.subscribers
            .register_subscriber(RegisteredSubscriber {
                trigger_id: observer_key(sink.id()),
                target: SubscriberTarget::Observer(sink),
                spec,
            })
            .await
            .map_err(|error| match error {
                RegisterError::InvalidConfig(message) => {
                    ObserveRejection::new(observe_codes::INVALID_ARGS, message)
                }
                RegisterError::AlreadyRegistered(message) => {
                    ObserveRejection::new(observe_codes::OBSERVER_ID_CONFLICT, message)
                }
            })
    }

    /// Unregisters the observer `id`: its consumer stops, and its backlog
    /// stays for a later observer with the same id, as a trigger's does.
    pub async fn remove(&self, id: &str) {
        self.subscribers.unregister(&observer_key(id)).await;
    }
}

#[async_trait]
impl ObserveHandler for OnMessageObservers {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        let spec = parse_args(obs.args_value())?;
        self.add(Arc::new(obs.clone()), spec).await
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.remove(obs.id()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{QueueAdapter, SwappableAdapter};
    use crate::adapters::builtin::BuiltinAdapter;
    use crate::store::{InMemoryStore, QueueStore};
    use crate::trigger::Invoker;
    use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
    use std::future::Future;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;
    use tokio::time::{sleep, Instant};

    /// A trigger target: records `(function_id, payload)` per delivery.
    #[derive(Default)]
    struct RecordingInvoker {
        calls: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl Invoker for RecordingInvoker {
        async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String> {
            self.calls
                .lock()
                .await
                .push((function_id.to_string(), payload));
            Ok(Some(json!({ "ok": true })))
        }
    }

    /// An observer: records each delivered `obs.call` payload, after
    /// failing the first `fail_first` calls with `failure`.
    struct FakeObserver {
        id: String,
        target: String,
        calls: Mutex<Vec<Value>>,
        fail_first: usize,
        failure: String,
        attempts: AtomicUsize,
    }

    impl FakeObserver {
        fn new(id: &str) -> Arc<Self> {
            Self::failing(id, 0, "")
        }

        fn failing(id: &str, fail_first: usize, failure: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                target: format!("{id}::target"),
                calls: Mutex::new(Vec::new()),
                fail_first,
                failure: failure.to_string(),
                attempts: AtomicUsize::new(0),
            })
        }

        async fn delivered(&self) -> Vec<Value> {
            self.calls.lock().await.clone()
        }
    }

    #[async_trait]
    impl ObserverSink for FakeObserver {
        fn id(&self) -> &str {
            &self.id
        }

        fn target(&self) -> &str {
            &self.target
        }

        async fn call(&self, payload: Value) -> Result<Option<Value>, String> {
            if self.attempts.fetch_add(1, Ordering::SeqCst) < self.fail_first {
                return Err(self.failure.clone());
            }
            self.calls.lock().await.push(payload);
            Ok(Some(json!({ "ok": true })))
        }
    }

    struct Fixture {
        store: Arc<dyn QueueStore>,
        adapter: Arc<SwappableAdapter>,
        invoker: Arc<RecordingInvoker>,
        handler: QueueTriggerHandler,
        observers: OnMessageObservers,
    }

    fn fixture() -> Fixture {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(RecordingInvoker::default());
        let builtin: Arc<dyn QueueAdapter> =
            Arc::new(BuiltinAdapter::new(store.clone(), invoker.clone()));
        let adapter = Arc::new(SwappableAdapter::new(builtin, "builtin"));
        let handler = QueueTriggerHandler::new(adapter.clone());
        let observers = OnMessageObservers::new(handler.clone());
        Fixture {
            store,
            adapter,
            invoker,
            handler,
            observers,
        }
    }

    fn spec(args: Value) -> SubscriberSpec {
        parse_args(&args).expect("valid args")
    }

    fn trigger_config(id: &str, function_id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: function_id.to_string(),
            config,
            metadata: None,
            namespace: None,
        }
    }

    async fn wait_until<F, Fut>(mut pred: F)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = bool>,
    {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if pred().await {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert!(pred().await, "condition did not become true before timeout");
    }

    #[test]
    fn args_accept_the_trigger_config_and_its_aliases() {
        let by_topic = spec(json!({ "topic": "orders", "maxRetries": 2, "backoffDelayMs": 5 }));
        assert_eq!(by_topic.queue, "orders");
        assert_eq!(by_topic.max_retries, Some(2));
        assert_eq!(by_topic.backoff_ms, Some(5));
        let by_queue = spec(json!({
            "queue": "orders",
            "condition_function_id": "orders::wanted",
            "queue_config": { "type": "fifo", "concurrency": 1 }
        }));
        assert_eq!(by_queue.queue, "orders");
        assert_eq!(
            by_queue.condition_function_id.as_deref(),
            Some("orders::wanted")
        );
        assert_eq!(
            by_queue.queue_config.unwrap().queue_mode.as_deref(),
            Some("fifo")
        );
    }

    #[test]
    fn args_without_a_topic_or_with_both_spellings_are_invalid_args() {
        for args in [
            json!({}),
            json!({ "topic": "a", "queue": "b" }),
            json!({ "topic": 7 }),
        ] {
            let rejection = parse_args(&args).unwrap_err();
            assert_eq!(rejection.code, observe_codes::INVALID_ARGS, "{args}");
        }
    }

    #[test]
    fn args_format_requires_queue_or_topic_and_documents_the_aliases() {
        let schema = args_format();
        assert!(schema.get("required").is_none());
        assert_eq!(
            schema["anyOf"],
            json!([{ "required": ["queue"] }, { "required": ["topic"] }])
        );
        let properties = schema["properties"].as_object().unwrap();
        for name in [
            "queue",
            "topic",
            "max_retries",
            "maxRetries",
            "backoff_ms",
            "backoffDelayMs",
            "condition_function_id",
            "queue_config",
        ] {
            assert!(properties.contains_key(name), "missing {name}");
        }
        assert_eq!(properties["topic"], properties["queue"]);
    }

    #[tokio::test]
    async fn an_empty_topic_is_rejected_by_the_shared_validation() {
        let f = fixture();
        let rejection = f
            .observers
            .add(FakeObserver::new("obs-1"), spec(json!({ "topic": " " })))
            .await
            .unwrap_err();
        assert_eq!(rejection.code, observe_codes::INVALID_ARGS);
        assert_eq!(
            rejection.message,
            "queue is required for queue::on-message observer"
        );
        // The trigger type keeps its own wording.
        let err = f
            .handler
            .register_trigger(trigger_config("t1", "fn", json!({ "queue": "" })))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("durable:subscriber trigger"));
        assert!(f.handler.registrations().await.is_empty());
    }

    #[test]
    fn direct_invocation_is_forbidden() {
        match refuse_invocation(QueueMessage(json!({ "forged": true }))) {
            Err(Error::Remote { code, message, .. }) => {
                assert_eq!(code, observe_codes::FORBIDDEN);
                assert!(message.contains("iii.observe"));
            }
            other => panic!("expected FORBIDDEN, got {other:?}"),
        }
    }

    #[test]
    fn the_message_schema_is_typed() {
        // The registry publish gate rejects an AnyValue request schema.
        let schema = serde_json::to_value(schemars::schema_for!(QueueMessage)).unwrap();
        assert!(schema.get("type").is_some(), "{schema}");
    }

    #[test]
    fn call_outcome_acks_results_and_filter_misses_and_fails_the_rest() {
        assert_eq!(
            call_outcome(Ok(json!({ "ok": true }))),
            Ok(Some(json!({ "ok": true })))
        );
        let remote = |code: &str| Error::Remote {
            code: code.to_string(),
            message: "m".to_string(),
            stacktrace: None,
        };
        assert_eq!(
            call_outcome(Err(remote(observe_codes::FILTERS_NOT_MATCHED))),
            Ok(None)
        );
        for code in [
            observe_codes::OBSERVER_NOT_ACTIVE,
            observe_codes::OVERFLOW,
            "TIMEOUT",
        ] {
            assert!(call_outcome(Err(remote(code))).is_err(), "{code}");
        }
        assert!(call_outcome(Err(Error::NotConnected)).is_err());
    }

    /// An observer and a trigger on one topic are two subscribers: each
    /// receives every message (fan-out), with the same payload, as two
    /// triggers with different targets do. Only registrations sharing a
    /// subscription key (two triggers on one function) share one consumer.
    #[tokio::test]
    async fn an_observer_and_a_trigger_on_one_topic_each_receive_every_message() {
        let f = fixture();
        f.handler
            .register_trigger(trigger_config(
                "t1",
                "orders::legacy",
                json!({ "queue": "orders" }),
            ))
            .await
            .unwrap();
        let observer = FakeObserver::new("obs-1");
        f.observers
            .add(observer.clone(), spec(json!({ "topic": "orders" })))
            .await
            .unwrap();
        // A second observer of the same topic is one more subscriber too.
        let second = FakeObserver::new("obs-2");
        f.observers
            .add(second.clone(), spec(json!({ "topic": "orders" })))
            .await
            .unwrap();

        for i in 0..3 {
            f.adapter
                .enqueue("orders", json!({ "n": i }), None, None)
                .await;
        }
        wait_until(|| async {
            f.invoker.calls.lock().await.len() == 3
                && observer.delivered().await.len() == 3
                && second.delivered().await.len() == 3
        })
        .await;

        let mut via_trigger: Vec<Value> = f
            .invoker
            .calls
            .lock()
            .await
            .iter()
            .map(|(function_id, payload)| {
                assert_eq!(function_id, "orders::legacy");
                payload.clone()
            })
            .collect();
        let mut via_observer = observer.delivered().await;
        via_trigger.sort_by_key(|p| p["n"].as_i64());
        via_observer.sort_by_key(|p| p["n"].as_i64());
        assert_eq!(via_trigger, via_observer, "the same payload either way");
        f.handler.shutdown().await;
    }

    #[tokio::test]
    async fn a_successful_call_acks_the_message() {
        let f = fixture();
        let observer = FakeObserver::new("obs-1");
        f.observers
            .add(observer.clone(), spec(json!({ "topic": "orders" })))
            .await
            .unwrap();
        f.adapter
            .enqueue("orders", json!({ "id": 1 }), None, None)
            .await;
        wait_until(|| async { observer.delivered().await.len() == 1 }).await;
        let queue = format!("orders::{}", observer_key("obs-1"));
        wait_until(|| async {
            let stats = f.store.topic_stats(&queue).await;
            stats.depth == 0 && stats.delivered == 1
        })
        .await;
        assert_eq!(f.adapter.dlq_count("orders").await.unwrap(), 0);
        f.handler.shutdown().await;
    }

    /// A failing target is retried with the observer's own backoff and
    /// then dead-lettered, after as many attempts as a trigger target with
    /// the same config.
    #[tokio::test]
    async fn a_failing_call_is_retried_then_dead_lettered_like_a_trigger_target() {
        let f = fixture();
        let observer = FakeObserver::failing("obs-1", usize::MAX, "TARGET_ERROR: boom");
        f.observers
            .add(
                observer.clone(),
                spec(json!({ "topic": "orders", "max_retries": 2, "backoff_ms": 1 })),
            )
            .await
            .unwrap();
        f.adapter
            .enqueue("orders", json!({ "id": 1 }), None, None)
            .await;
        wait_until(|| async { f.adapter.dlq_count("orders").await.unwrap_or(0) == 1 }).await;
        let dead = f.adapter.dlq_peek("orders", 0, 10).await.unwrap();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0]["attempts"], json!(2));
        assert_eq!(observer.attempts.load(Ordering::SeqCst), 2);
        assert!(observer.delivered().await.is_empty());
        f.handler.shutdown().await;
    }

    /// A suspended observer (its worker briefly gone) fails its calls with
    /// `OBSERVER_NOT_ACTIVE`: the message stays queued and is retried, and
    /// is delivered once the observer is back within the retry budget.
    #[tokio::test]
    async fn a_suspended_observer_keeps_its_messages_and_gets_them_on_retry() {
        let f = fixture();
        let observer = FakeObserver::failing(
            "obs-1",
            2,
            "OBSERVER_NOT_ACTIVE: observer obs-1 is not active",
        );
        f.observers
            .add(
                observer.clone(),
                spec(json!({ "topic": "orders", "max_retries": 5, "backoff_ms": 1 })),
            )
            .await
            .unwrap();
        f.adapter
            .enqueue("orders", json!({ "id": 1 }), None, None)
            .await;
        wait_until(|| async { observer.delivered().await.len() == 1 }).await;
        assert_eq!(observer.delivered().await, vec![json!({ "id": 1 })]);
        assert_eq!(observer.attempts.load(Ordering::SeqCst), 3);
        assert_eq!(f.adapter.dlq_count("orders").await.unwrap(), 0);
        f.handler.shutdown().await;
    }

    /// Removing an observer stops its consumer as unregistering a trigger
    /// does: later messages stay in its queue, and an observer with the same
    /// id picks them up.
    #[tokio::test]
    async fn removal_stops_delivery_and_keeps_the_backlog_for_the_same_id() {
        let f = fixture();
        f.handler
            .register_trigger(trigger_config(
                "t1",
                "orders::legacy",
                json!({ "queue": "orders" }),
            ))
            .await
            .unwrap();
        let observer = FakeObserver::new("obs-1");
        f.observers
            .add(observer.clone(), spec(json!({ "topic": "orders" })))
            .await
            .unwrap();
        f.adapter
            .enqueue("orders", json!({ "n": 1 }), None, None)
            .await;
        wait_until(|| async { observer.delivered().await.len() == 1 }).await;

        f.observers.remove("obs-1").await;
        assert_eq!(
            f.handler.registrations().await.len(),
            1,
            "the trigger stays"
        );
        f.adapter
            .enqueue("orders", json!({ "n": 2 }), None, None)
            .await;
        wait_until(|| async { f.invoker.calls.lock().await.len() == 2 }).await;
        sleep(Duration::from_millis(100)).await;
        assert_eq!(
            observer.delivered().await.len(),
            1,
            "no delivery after removal"
        );

        // Back under the same id: its queue was kept, but the topic no longer
        // fans out to it while it is gone, as for a trigger.
        let again = FakeObserver::new("obs-1");
        f.observers
            .add(again.clone(), spec(json!({ "topic": "orders" })))
            .await
            .unwrap();
        f.adapter
            .enqueue("orders", json!({ "n": 3 }), None, None)
            .await;
        wait_until(|| async { again.delivered().await.len() == 1 }).await;
        assert_eq!(again.delivered().await, vec![json!({ "n": 3 })]);
        f.handler.shutdown().await;
    }

    #[tokio::test]
    async fn an_observer_id_never_collides_with_a_trigger_id() {
        let f = fixture();
        f.handler
            .register_trigger(trigger_config(
                "same",
                "orders::legacy",
                json!({ "queue": "orders" }),
            ))
            .await
            .unwrap();
        f.observers
            .add(
                FakeObserver::new("same"),
                spec(json!({ "topic": "orders" })),
            )
            .await
            .unwrap();
        assert_eq!(f.handler.registrations().await.len(), 2);
        let duplicate = f
            .observers
            .add(
                FakeObserver::new("same"),
                spec(json!({ "topic": "orders" })),
            )
            .await
            .unwrap_err();
        assert_eq!(duplicate.code, observe_codes::OBSERVER_ID_CONFLICT);
        // Removing the observer leaves the trigger of the same id alone.
        f.observers.remove("same").await;
        let left = f.handler.registrations().await;
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].target.function_id(), "orders::legacy");
        f.handler.shutdown().await;
    }

    /// A configuration hot-swap re-attaches observers to the new adapter
    /// along with the triggers.
    #[tokio::test]
    async fn an_adapter_swap_resubscribes_observers() {
        let f = fixture();
        let observer = FakeObserver::new("obs-1");
        f.observers
            .add(observer.clone(), spec(json!({ "topic": "orders" })))
            .await
            .unwrap();

        let next_store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let next: Arc<dyn QueueAdapter> =
            Arc::new(BuiltinAdapter::new(next_store, f.invoker.clone()));
        let previous = f.adapter.current().await;
        f.adapter.replace(next, "builtin").await;
        previous.shutdown().await;
        f.handler.resubscribe_all().await;

        f.adapter
            .enqueue("orders", json!({ "after": "swap" }), None, None)
            .await;
        wait_until(|| async { observer.delivered().await.len() == 1 }).await;
        f.handler.shutdown().await;
    }
}
