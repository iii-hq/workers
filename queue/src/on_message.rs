//! `queue::on-message`: one call per message for one subscriber of a durable
//! queue topic.
//!
//! The function is private and its handler returns the message it is called
//! with. Its observers are the topic's subscribers: their args are a
//! [`SubscriberSpec`] (`{ topic, max_retries?, backoff_ms?, queue_config?, group? }`).
//!
//! - **Added.** [`OnMessageObservers`] files each observer with
//!   [`Subscribers`], which opens its consumer on the adapter before the SDK
//!   acks it (`INVALID_ARGS` without a topic, `OBSERVER_ID_CONFLICT` for an id
//!   already subscribed).
//! - **Removed.** The observer's consumer stops; its durable queue and backlog
//!   stay for a later observer with the same id.
//! - **Match.** By keys: an observer is filed under its id, and a call
//!   matches the subscriber its metadata names (`{ "subscriber": <id> }`).
//! - **Deliver.** The consumer loops call the function once per message with
//!   that metadata ([`crate::subscribers::IiiInvoker`]). The deliver hook
//!   ([`deliver_message`]) calls through the matched observer, and the
//!   target's result is the call's result: it acks the message, and an error
//!   retries it with backoff and then dead-letters it. The engine bounds the
//!   call through the observer by `observers.activation_timeout_ms`; a target
//!   slower than that fails the delivery with `TIMEOUT`. A subscriber that is
//!   not active fails the call with `OBSERVER_NOT_ACTIVE` (retried, then
//!   dead-lettered); a message its filters reject is acked with `null`.

use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::observe_codes;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{
    AddedHook, Call, CallRequest, DeliverHook, IIIClient, Match, ObserveRejection, Observer,
    RegisterFunction, RemovedHook,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::subscribers::{
    RegisterError, RegisteredSubscriber, SubscriberSpec, Subscribers,
    FUNCTION_QUEUE_INVOCATION_TIMEOUT_MS,
};

/// The function a queue topic's messages are delivered through.
pub const ON_MESSAGE_FN: &str = "queue::on-message";

/// The call metadata key naming the subscriber (observer id) a message is for.
pub const SUBSCRIBER_METADATA_KEY: &str = "subscriber";

pub const ON_MESSAGE_DESCRIPTION: &str =
    "One call per message published to a durable queue topic, for each subscriber of the topic. \
     Observe it with args { topic, max_retries?, backoff_ms?, queue_config?, group? }: each \
     observer is a subscriber with its own durable queue, retries and dead-letter queue, except \
     that observers naming the same group share one queue and handle each message once. The \
     target receives the published data; returning acks the message, and an error retries it \
     with backoff and then moves it to the topic's DLQ. The target is called through its \
     observer, so the engine's observers.activation_timeout_ms (30 s by default) bounds each \
     delivery: a target that has not answered by then fails it with TIMEOUT and the message is \
     retried; raise that engine setting for targets that run longer. Filters read the message \
     as input; a message they reject is acked without a delivery. Only the queue worker calls \
     it.";

/// Sets the hooks of `queue::on-message` and registers it: private, its
/// handler returns the message, its observers' args are [`args_format`].
/// Hooks are set before the registration so they see every observer.
pub fn register(iii: &IIIClient, subscribers: Subscribers) -> FunctionRef {
    let observers = OnMessageObservers::new(subscribers);
    iii.on_observer_added_for(ON_MESSAGE_FN, observers.clone())
        .on_observer_removed_for(ON_MESSAGE_FN, observers)
        .on_observer_match_for(
            ON_MESSAGE_FN,
            Match::keys(|obs| obs.id().into(), |call| subscriber_of(call).into()),
        )
        .on_observer_deliver_for(ON_MESSAGE_FN, OnMessageDelivery { iii: iii.clone() });
    iii.register_function(
        ON_MESSAGE_FN,
        RegisterFunction::new(on_message)
            .description(ON_MESSAGE_DESCRIPTION)
            .args_format_value(args_format())
            .private(),
    )
}

/// What a `queue::on-message` call carries and its target receives: the
/// data published to the topic, unchanged. Published data is not always an
/// object, so this keeps a lossless `Value` while publishing a real schema
/// (the registry publish gate rejects `AnyValue`).
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// The function's handler: it returns the message it is called with, which
/// is what observers observe (`input` and `output` alike).
fn on_message(message: QueueMessage) -> Result<QueueMessage, Error> {
    Ok(message)
}

/// The observers' `args_format`: [`SubscriberSpec`]'s schema plus the
/// aliases its serde config accepts (`topic` for `queue`, `maxRetries`,
/// `backoffDelayMs`), with one of `queue` / `topic` required and no other
/// field. The engine refuses any other args (`INVALID_ARGS`); [`parse_args`]
/// and [`Subscribers::register`] do the rest.
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

/// An observer's args, parsed.
pub fn parse_args(args: &Value) -> Result<SubscriberSpec, ObserveRejection> {
    serde_json::from_value(args.clone()).map_err(|e| {
        ObserveRejection::new(
            observe_codes::INVALID_ARGS,
            format!("invalid {ON_MESSAGE_FN} args: {e}"),
        )
    })
}

/// The subscriber (observer id) a call is for: its metadata's `subscriber`.
pub fn subscriber_of(call: &Call) -> Option<&str> {
    call.metadata
        .as_ref()?
        .get(SUBSCRIBER_METADATA_KEY)?
        .as_str()
}

/// The subscriber matched to one call, as [`deliver_message`] sees it: an
/// [`Observer`] called through the engine in production, a fake in tests.
#[async_trait]
pub trait MatchedSubscriber: Send + Sync {
    /// Whether the observer's `on` and filters admit the call.
    fn matches(&self, call: &Call) -> bool;

    /// Call the observer's target with `payload` and return its result.
    async fn call_through(&self, payload: Value) -> Result<Value, Error>;
}

/// Deliver one message to the subscriber matched to it, and return what the
/// consumer loop gets back: the target's result acks the message and its
/// error fails it. No matched subscriber (removed while the message was in
/// flight, or not yet active) is `OBSERVER_NOT_ACTIVE`, so the message is
/// retried and then dead-lettered; a message the subscriber's filters reject
/// is acked with `null` and never reaches its target.
pub async fn deliver_message(
    call: &Call,
    subscriber: Option<&dyn MatchedSubscriber>,
) -> Result<Value, Error> {
    let Some(subscriber) = subscriber else {
        return Err(ObserveRejection::new(
            observe_codes::OBSERVER_NOT_ACTIVE,
            format!(
                "{ON_MESSAGE_FN} subscriber '{}' is not active",
                subscriber_of(call).unwrap_or_default()
            ),
        )
        .into());
    };
    if !subscriber.matches(call) {
        return Ok(Value::Null);
    }
    subscriber.call_through(call.input.clone()).await
}

/// An [`Observer`] of `queue::on-message`, called through with this worker's
/// client.
struct ThroughObserver<'a> {
    iii: &'a IIIClient,
    obs: &'a Observer,
}

#[async_trait]
impl MatchedSubscriber for ThroughObserver<'_> {
    fn matches(&self, call: &Call) -> bool {
        self.obs.matches(call)
    }

    async fn call_through(&self, payload: Value) -> Result<Value, Error> {
        // The call carries the consumer's 30-minute budget for this
        // function; the engine ends a call through an observer with
        // `TIMEOUT` after `observers.activation_timeout_ms`, whichever is
        // shorter.
        self.iii
            .call(
                CallRequest::observer(self.obs, payload)
                    .timeout(Duration::from_millis(FUNCTION_QUEUE_INVOCATION_TIMEOUT_MS)),
            )
            .await
    }
}

/// The deliver hook of `queue::on-message`.
#[derive(Clone)]
pub struct OnMessageDelivery {
    iii: IIIClient,
}

#[async_trait]
impl DeliverHook for OnMessageDelivery {
    async fn on_deliver(&self, call: Call, observers: Vec<Observer>) -> Result<Value, Error> {
        // Matched by keys on the observer id: at most one observer.
        let through = observers.first().map(|obs| ThroughObserver {
            iii: &self.iii,
            obs,
        });
        deliver_message(
            &call,
            through
                .as_ref()
                .map(|through| through as &dyn MatchedSubscriber),
        )
        .await
    }
}

/// The added and removed hooks of `queue::on-message`: each observer is one
/// subscriber of [`Subscribers`], by observer id.
#[derive(Clone)]
pub struct OnMessageObservers {
    subscribers: Subscribers,
}

impl OnMessageObservers {
    pub fn new(subscribers: Subscribers) -> Self {
        Self { subscribers }
    }

    /// Subscribe the observer `id` with its `args`. Its consumer starts here,
    /// before the SDK acks the observer, so no message published after the
    /// observer is active is missed.
    pub async fn add(&self, id: &str, args: &Value) -> Result<(), ObserveRejection> {
        let spec = parse_args(args)?;
        self.subscribers
            .register(RegisteredSubscriber {
                id: id.to_string(),
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

    /// Unsubscribe the observer `id`: its consumer stops, and its backlog
    /// stays for a later observer with the same id.
    pub async fn remove(&self, id: &str) {
        self.subscribers.unregister(id).await;
    }
}

#[async_trait]
impl AddedHook for OnMessageObservers {
    async fn on_added(&self, obs: Observer) -> Result<(), ObserveRejection> {
        self.add(obs.id(), obs.args_value()).await
    }
}

#[async_trait]
impl RemovedHook for OnMessageObservers {
    async fn on_removed(&self, obs: Observer) {
        self.remove(obs.id()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{QueueAdapter, SwappableAdapter};
    use crate::adapters::builtin::BuiltinAdapter;
    use crate::store::{InMemoryStore, QueueStore};
    use crate::subscribers::Invoker;
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Mutex;
    use tokio::time::{sleep, Instant};

    fn remote(code: &str, message: &str) -> Error {
        Error::Remote {
            code: code.to_string(),
            message: message.to_string(),
            stacktrace: None,
        }
    }

    /// A call of `queue::on-message` for `subscriber`, as the SDK hands it to
    /// the deliver hook: the handler returned its input.
    fn message_call(subscriber: &str, payload: Value) -> Call {
        Call {
            function_id: ON_MESSAGE_FN.to_string(),
            output: Some(payload.clone()),
            input: payload,
            metadata: Some(json!({ SUBSCRIBER_METADATA_KEY: subscriber })),
            ..Call::default()
        }
    }

    /// An observer: its filters reject a message whose `skip` is `true`; it
    /// records each payload called through it, after failing the first
    /// `fail_first` calls with `failure`.
    struct FakeObserver {
        id: String,
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
    impl MatchedSubscriber for FakeObserver {
        fn matches(&self, call: &Call) -> bool {
            call.input.get("skip").and_then(Value::as_bool) != Some(true)
        }

        async fn call_through(&self, payload: Value) -> Result<Value, Error> {
            if self.attempts.fetch_add(1, Ordering::SeqCst) < self.fail_first {
                let (code, message) = self
                    .failure
                    .split_once(": ")
                    .unwrap_or((self.failure.as_str(), ""));
                return Err(remote(code, message));
            }
            self.calls.lock().await.push(payload);
            Ok(json!({ "ok": true }))
        }
    }

    /// The SDK and the engine between a consumer and its observer: each
    /// delivery is a call of `queue::on-message` naming the subscriber, the
    /// observer filed under that id is matched (by keys), and the deliver
    /// hook runs.
    #[derive(Default)]
    struct FakeEngine {
        observers: StdMutex<HashMap<String, Arc<FakeObserver>>>,
    }

    #[async_trait]
    impl Invoker for FakeEngine {
        async fn call(&self, function_id: &str, _payload: Value) -> Result<Option<Value>, String> {
            Err(format!("unexpected call of {function_id}"))
        }

        async fn deliver(&self, subscriber: &str, payload: Value) -> Result<Option<Value>, String> {
            let call = message_call(subscriber, payload);
            let matched = self.observers.lock().unwrap().get(subscriber).cloned();
            deliver_message(
                &call,
                matched
                    .as_deref()
                    .map(|observer| observer as &dyn MatchedSubscriber),
            )
            .await
            .map(Some)
            .map_err(|error| error.to_string())
        }
    }

    struct Fixture {
        store: Arc<dyn QueueStore>,
        adapter: Arc<SwappableAdapter>,
        engine: Arc<FakeEngine>,
        subscribers: Subscribers,
        observers: OnMessageObservers,
    }

    impl Fixture {
        /// The observer is added: the engine knows it, and its hooks run.
        async fn add(&self, observer: &Arc<FakeObserver>, args: Value) {
            self.try_add(observer, args)
                .await
                .expect("observer accepted");
        }

        async fn try_add(
            &self,
            observer: &Arc<FakeObserver>,
            args: Value,
        ) -> Result<(), ObserveRejection> {
            self.observers.add(&observer.id, &args).await?;
            self.engine
                .observers
                .lock()
                .unwrap()
                .insert(observer.id.clone(), observer.clone());
            Ok(())
        }

        async fn remove(&self, id: &str) {
            self.engine.observers.lock().unwrap().remove(id);
            self.observers.remove(id).await;
        }
    }

    fn fixture() -> Fixture {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let engine = Arc::new(FakeEngine::default());
        let builtin: Arc<dyn QueueAdapter> =
            Arc::new(BuiltinAdapter::new(store.clone(), engine.clone()));
        let adapter = Arc::new(SwappableAdapter::new(builtin, "builtin"));
        let subscribers = Subscribers::new(adapter.clone());
        let observers = OnMessageObservers::new(subscribers.clone());
        Fixture {
            store,
            adapter,
            engine,
            subscribers,
            observers,
        }
    }

    fn spec(args: Value) -> SubscriberSpec {
        parse_args(&args).expect("valid args")
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
    fn args_accept_the_topic_and_its_aliases() {
        let by_topic = spec(json!({ "topic": "orders", "maxRetries": 2, "backoffDelayMs": 5 }));
        assert_eq!(by_topic.queue, "orders");
        assert_eq!(by_topic.max_retries, Some(2));
        assert_eq!(by_topic.backoff_ms, Some(5));
        let by_queue = spec(json!({
            "queue": "orders",
            "group": "billing",
            "queue_config": { "type": "fifo", "concurrency": 1 }
        }));
        assert_eq!(by_queue.queue, "orders");
        assert_eq!(by_queue.group.as_deref(), Some("billing"));
        assert_eq!(
            by_queue.queue_config.unwrap().queue_mode.as_deref(),
            Some("fifo")
        );
    }

    #[test]
    fn args_without_a_topic_with_both_spellings_or_unknown_fields_are_invalid_args() {
        for args in [
            json!({}),
            json!({ "topic": "a", "queue": "b" }),
            json!({ "topic": 7 }),
            json!({ "topic": "a", "condition_function_id": "orders::wanted" }),
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
        assert_eq!(schema["additionalProperties"], json!(false));
        let properties = schema["properties"].as_object().unwrap();
        let mut names: Vec<_> = properties.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "backoffDelayMs",
                "backoff_ms",
                "group",
                "maxRetries",
                "max_retries",
                "queue",
                "queue_config",
                "topic",
            ]
        );
        assert_eq!(properties["topic"], properties["queue"]);
    }

    #[test]
    fn the_handler_returns_the_message_it_is_called_with() {
        let message = json!({ "order": 7, "items": ["a"] });
        let out = on_message(QueueMessage(message.clone())).unwrap();
        assert_eq!(out.0, message);
        assert_eq!(
            on_message(QueueMessage(json!(null))).unwrap().0,
            json!(null)
        );
    }

    #[test]
    fn the_message_schema_is_typed() {
        // The registry publish gate rejects an AnyValue request schema.
        let schema = serde_json::to_value(schemars::schema_for!(QueueMessage)).unwrap();
        assert!(schema.get("type").is_some(), "{schema}");
    }

    #[test]
    fn a_call_names_its_subscriber_in_its_metadata() {
        assert_eq!(
            subscriber_of(&message_call("obs-1", json!(1))),
            Some("obs-1")
        );
        assert_eq!(subscriber_of(&Call::default()), None);
        let other = Call {
            metadata: Some(json!({ "subscriber": 7 })),
            ..Call::default()
        };
        assert_eq!(subscriber_of(&other), None);
    }

    #[tokio::test]
    async fn deliver_calls_through_the_matched_subscriber_and_returns_its_result() {
        let observer = FakeObserver::new("obs-1");
        let out = deliver_message(
            &message_call("obs-1", json!({ "id": 1 })),
            Some(observer.as_ref()),
        )
        .await
        .unwrap();
        assert_eq!(out, json!({ "ok": true }));
        assert_eq!(observer.delivered().await, vec![json!({ "id": 1 })]);
    }

    #[tokio::test]
    async fn deliver_without_a_subscriber_is_observer_not_active() {
        match deliver_message(&message_call("gone", json!({ "id": 1 })), None).await {
            Err(Error::Remote { code, message, .. }) => {
                assert_eq!(code, observe_codes::OBSERVER_NOT_ACTIVE);
                assert!(message.contains("'gone'"), "{message}");
            }
            other => panic!("expected OBSERVER_NOT_ACTIVE, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn deliver_acks_a_filter_miss_without_calling_the_target() {
        let observer = FakeObserver::new("obs-1");
        let out = deliver_message(
            &message_call("obs-1", json!({ "skip": true })),
            Some(observer.as_ref()),
        )
        .await
        .unwrap();
        assert_eq!(out, Value::Null);
        assert_eq!(observer.attempts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn deliver_passes_the_target_error_through() {
        let observer = FakeObserver::failing("obs-1", 1, "TARGET_ERROR: boom");
        match deliver_message(&message_call("obs-1", json!(1)), Some(observer.as_ref())).await {
            Err(Error::Remote { code, .. }) => assert_eq!(code, "TARGET_ERROR"),
            other => panic!("expected the target's error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_empty_topic_is_invalid_args() {
        let f = fixture();
        let rejection = f
            .try_add(&FakeObserver::new("obs-1"), json!({ "topic": " " }))
            .await
            .unwrap_err();
        assert_eq!(rejection.code, observe_codes::INVALID_ARGS);
        assert_eq!(
            rejection.message,
            "topic is required for a queue::on-message observer"
        );
        assert!(f.subscribers.registrations().await.is_empty());
    }

    #[tokio::test]
    async fn a_duplicate_observer_id_is_observer_id_conflict() {
        let f = fixture();
        f.add(&FakeObserver::new("same"), json!({ "topic": "orders" }))
            .await;
        let duplicate = f
            .observers
            .add("same", &json!({ "topic": "orders" }))
            .await
            .unwrap_err();
        assert_eq!(duplicate.code, observe_codes::OBSERVER_ID_CONFLICT);
        assert_eq!(f.subscribers.registrations().await.len(), 1);
        f.subscribers.shutdown().await;
    }

    /// Two observers of one topic are two subscribers: each receives every
    /// message (fan-out), with the same payload.
    #[tokio::test]
    async fn each_observer_of_a_topic_receives_every_message() {
        let f = fixture();
        let first = FakeObserver::new("obs-1");
        let second = FakeObserver::new("obs-2");
        f.add(&first, json!({ "topic": "orders" })).await;
        f.add(&second, json!({ "topic": "orders" })).await;

        for i in 0..3 {
            f.adapter
                .enqueue("orders", json!({ "n": i }), None, None)
                .await;
        }
        wait_until(|| async {
            first.delivered().await.len() == 3 && second.delivered().await.len() == 3
        })
        .await;

        let mut via_first = first.delivered().await;
        let mut via_second = second.delivered().await;
        via_first.sort_by_key(|p| p["n"].as_i64());
        via_second.sort_by_key(|p| p["n"].as_i64());
        assert_eq!(via_first, via_second, "the same payload to each subscriber");
        f.subscribers.shutdown().await;
    }

    #[tokio::test]
    async fn a_successful_call_acks_the_message() {
        let f = fixture();
        let observer = FakeObserver::new("obs-1");
        f.add(&observer, json!({ "topic": "orders" })).await;
        f.adapter
            .enqueue("orders", json!({ "id": 1 }), None, None)
            .await;
        wait_until(|| async { observer.delivered().await.len() == 1 }).await;
        wait_until(|| async {
            let stats = f.store.topic_stats("orders::observer:obs-1").await;
            stats.depth == 0 && stats.delivered == 1
        })
        .await;
        assert_eq!(f.adapter.dlq_count("orders").await.unwrap(), 0);
        f.subscribers.shutdown().await;
    }

    /// A message the observer's filters reject is acked with no delivery
    /// and no retry.
    #[tokio::test]
    async fn a_filter_miss_acks_the_message() {
        let f = fixture();
        let observer = FakeObserver::new("obs-1");
        f.add(&observer, json!({ "topic": "orders", "max_retries": 2 }))
            .await;
        f.adapter
            .enqueue("orders", json!({ "skip": true }), None, None)
            .await;
        wait_until(|| async {
            f.store
                .topic_stats("orders::observer:obs-1")
                .await
                .delivered
                == 1
        })
        .await;
        assert_eq!(observer.attempts.load(Ordering::SeqCst), 0);
        assert_eq!(f.adapter.dlq_count("orders").await.unwrap(), 0);
        f.subscribers.shutdown().await;
    }

    /// A failing target is retried with the observer's own backoff and
    /// then dead-lettered.
    #[tokio::test]
    async fn a_failing_target_is_retried_then_dead_lettered() {
        let f = fixture();
        let observer = FakeObserver::failing("obs-1", usize::MAX, "TARGET_ERROR: boom");
        f.add(
            &observer,
            json!({ "topic": "orders", "max_retries": 2, "backoff_ms": 1 }),
        )
        .await;
        f.adapter
            .enqueue("orders", json!({ "id": 1 }), None, None)
            .await;
        wait_until(|| async { f.adapter.dlq_count("orders").await.unwrap_or(0) == 1 }).await;
        let dead = f.adapter.dlq_peek("orders", 0, 10).await.unwrap();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0]["attempts"], json!(2));
        assert_eq!(observer.attempts.load(Ordering::SeqCst), 2);
        assert!(observer.delivered().await.is_empty());
        f.subscribers.shutdown().await;
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
        f.add(
            &observer,
            json!({ "topic": "orders", "max_retries": 5, "backoff_ms": 1 }),
        )
        .await;
        f.adapter
            .enqueue("orders", json!({ "id": 1 }), None, None)
            .await;
        wait_until(|| async { observer.delivered().await.len() == 1 }).await;
        assert_eq!(observer.delivered().await, vec![json!({ "id": 1 })]);
        assert_eq!(observer.attempts.load(Ordering::SeqCst), 3);
        assert_eq!(f.adapter.dlq_count("orders").await.unwrap(), 0);
        f.subscribers.shutdown().await;
    }

    /// Removing an observer stops its consumer: later messages reach the
    /// subscribers that stay, and an observer back under the same id gets
    /// its own queue again.
    #[tokio::test]
    async fn removal_stops_delivery_and_keeps_the_queue_for_the_same_id() {
        let f = fixture();
        let stays = FakeObserver::new("obs-stays");
        let leaves = FakeObserver::new("obs-1");
        f.add(&stays, json!({ "topic": "orders" })).await;
        f.add(&leaves, json!({ "topic": "orders" })).await;
        f.adapter
            .enqueue("orders", json!({ "n": 1 }), None, None)
            .await;
        wait_until(|| async { leaves.delivered().await.len() == 1 }).await;

        f.remove("obs-1").await;
        assert_eq!(f.subscribers.registrations().await.len(), 1);
        f.adapter
            .enqueue("orders", json!({ "n": 2 }), None, None)
            .await;
        wait_until(|| async { stays.delivered().await.len() == 2 }).await;
        sleep(Duration::from_millis(100)).await;
        assert_eq!(
            leaves.delivered().await.len(),
            1,
            "no delivery after removal"
        );

        // Back under the same id: its queue was kept, but the topic does not
        // fan out to it while it is gone.
        let again = FakeObserver::new("obs-1");
        f.add(&again, json!({ "topic": "orders" })).await;
        f.adapter
            .enqueue("orders", json!({ "n": 3 }), None, None)
            .await;
        wait_until(|| async { again.delivered().await.len() == 1 }).await;
        assert_eq!(again.delivered().await, vec![json!({ "n": 3 })]);
        f.subscribers.shutdown().await;
    }

    /// The members of a group share one queue: each message reaches one of
    /// them.
    #[tokio::test]
    async fn a_group_handles_each_message_once() {
        let f = fixture();
        let a = FakeObserver::new("obs-a");
        let b = FakeObserver::new("obs-b");
        let group = json!({ "topic": "orders", "group": "billing" });
        f.add(&a, group.clone()).await;
        f.add(&b, group).await;
        for i in 0..4 {
            f.adapter
                .enqueue("orders", json!({ "n": i }), None, None)
                .await;
        }
        wait_until(|| async { a.delivered().await.len() + b.delivered().await.len() == 4 }).await;
        sleep(Duration::from_millis(100)).await;
        assert_eq!(a.delivered().await.len() + b.delivered().await.len(), 4);
        f.subscribers.shutdown().await;
    }

    /// A configuration hot-swap re-attaches the subscribers to the new
    /// adapter.
    #[tokio::test]
    async fn an_adapter_swap_resubscribes_observers() {
        let f = fixture();
        let observer = FakeObserver::new("obs-1");
        f.add(&observer, json!({ "topic": "orders" })).await;

        let next_store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let next: Arc<dyn QueueAdapter> =
            Arc::new(BuiltinAdapter::new(next_store, f.engine.clone()));
        let previous = f.adapter.current().await;
        f.adapter.replace(next, "builtin").await;
        previous.shutdown().await;
        f.subscribers.resubscribe_all().await;

        f.adapter
            .enqueue("orders", json!({ "after": "swap" }), None, None)
            .await;
        wait_until(|| async { observer.delivered().await.len() == 1 }).await;
        f.subscribers.shutdown().await;
    }
}
