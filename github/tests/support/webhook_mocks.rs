//! Contract mocks, not the production queue adapter/cloudflared process.
//! Queue ACK means accepted; dispatch is explicit so assertions can inspect the
//! real SQLite inbox before the real registered process handler consumes it.
//! Each mock source is a private function whose hooks keep its observers. A
//! mock delivers by calling its own function. The queue mock's deliver hook
//! calls through the observer the call is for, as the queue worker does; the
//! tunnel mock has no deliver hook, so iii delivers each call to every
//! observer whose filters match, with that observer's `deliver`, as for the
//! quick-tunnel worker.
//! Like the queue worker, the queue keeps one durable subscription per
//! observer outside a group (`observer:<id>`) and one per group (`@<group>`);
//! a subscription and its backlog outlive the observers that consumed it.
use iii_sdk::{
    Call, CallRequest, Error, IIIClient, Match, ObserveRejection, Observer, RegisterFunction,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

pub const PROVIDER: &str = "integration-provider";
const ON_MESSAGE: &str = "queue::on-message";
const ON_CHANGED: &str = "quick-tunnel::on-changed";
pub const CONSUMER: &str = "integration-consumer";

/// The observers of one mock source, kept by its added and removed hooks.
#[derive(Clone, Default)]
pub struct Observers(Arc<Mutex<Vec<Observer>>>);
impl Observers {
    /// Hosts `function_id` as a private source whose handler returns the event,
    /// taking args only when `args_format` is given.
    fn host(
        &self,
        iii: &IIIClient,
        function_id: &str,
        description: &str,
        args_format: Option<Value>,
    ) {
        let added = self.clone();
        iii.on_observer_added_for(function_id, move |obs: Observer| {
            let added = added.clone();
            async move {
                added.0.lock().unwrap().push(obs);
                Ok::<(), ObserveRejection>(())
            }
        });
        let removed = self.clone();
        iii.on_observer_removed_for(function_id, move |obs: Observer| {
            let removed = removed.clone();
            async move {
                removed.0.lock().unwrap().retain(|o| o.id() != obs.id());
            }
        });
        let mut registration = RegisterFunction::new(|event: Value| Ok::<_, Error>(event))
            .description(description)
            .private();
        if let Some(schema) = args_format {
            registration = registration.args_format_value(schema);
        }
        iii.register_function(function_id, registration);
    }
    pub fn values(&self) -> Vec<Observer> {
        self.0.lock().unwrap().clone()
    }
}

// Same wire contract as the queue worker's SubscriberSpec: the top-level
// max_retries/backoff_ms are supported, not misclassified as invalid nesting.
#[derive(Deserialize)]
struct SubscriberSpec {
    #[serde(alias = "topic")]
    queue: String,
    max_retries: Option<u32>,
    backoff_ms: Option<u64>,
    group: Option<String>,
}
impl SubscriberSpec {
    fn of(obs: &Observer) -> Self {
        serde_json::from_value(obs.args_value().clone()).unwrap()
    }
    /// The durable subscription the observer consumes, as the queue worker
    /// keys it.
    fn subscription(&self, obs: &Observer) -> String {
        match self.group.as_deref().filter(|g| !g.is_empty()) {
            Some(group) => format!("@{group}"),
            None => format!("observer:{}", obs.id()),
        }
    }
}
/// One durable subscription of a topic and its backlog.
struct Subscription {
    topic: String,
    backlog: VecDeque<Value>,
}
#[derive(Clone, Default)]
pub struct Queue {
    pub observers: Observers,
    subscriptions: Arc<Mutex<BTreeMap<String, Subscription>>>,
    pub fail: Arc<AtomicBool>,
    pub failed_publications: Arc<AtomicUsize>,
    pub published: Arc<AtomicUsize>,
}
impl Queue {
    pub fn register(&self, iii: &IIIClient) {
        // As the queue worker: each call carries the subscriber it is for in
        // metadata.subscriber, matched by key; the deliver hook hands it to
        // that observer's target, whose result acks it and whose error leaves
        // it queued. A subscriber whose filters reject it acks it with null.
        iii.on_observer_match_for(
            ON_MESSAGE,
            Match::keys(
                |obs| obs.id().into(),
                |call| {
                    call.metadata
                        .as_ref()
                        .and_then(|m| m["subscriber"].as_str())
                        .into()
                },
            ),
        );
        let engine = iii.clone();
        iii.on_observer_deliver_for(ON_MESSAGE, move |call: Call, observers: Vec<Observer>| {
            let engine = engine.clone();
            async move {
                let Some(obs) = observers.first() else {
                    return Err(Error::from(ObserveRejection::new(
                        "OBSERVER_NOT_ACTIVE",
                        "the subscriber is not active",
                    )));
                };
                if !obs.matches(&call) {
                    return Ok(Value::Null);
                }
                engine.call(through(obs, call.input)).await
            }
        });
        self.observers.host(
            iii,
            ON_MESSAGE,
            "Deterministic local queue",
            Some(json!({"type":"object"})),
        );
        let queue = self.clone();
        iii.register_function(
            "iii::durable::publish",
            RegisterFunction::new(move |input: Value| {
                let topic = input
                    .get("topic")
                    .or_else(|| input.get("queue"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| Error::Handler("Topic is not set".into()))?;
                let data = input
                    .get("data")
                    .ok_or_else(|| Error::Handler("data is required".into()))?;
                if queue.fail.load(Ordering::SeqCst) {
                    queue.failed_publications.fetch_add(1, Ordering::SeqCst);
                    return Err(Error::Handler(
                        "injected durable publication failure".into(),
                    ));
                }
                // Every subscription of the topic gets the message: those
                // of its current observers and those they left.
                let mut subscriptions = queue.subscriptions.lock().unwrap();
                for obs in queue.observers.values() {
                    let spec = SubscriberSpec::of(&obs);
                    if spec.queue == topic {
                        subscriptions
                            .entry(spec.subscription(&obs))
                            .or_insert_with(|| Subscription {
                                topic: topic.into(),
                                backlog: VecDeque::new(),
                            });
                    }
                }
                let mut delivered = false;
                for subscription in subscriptions.values_mut().filter(|s| s.topic == topic) {
                    subscription.backlog.push_back(data.clone());
                    delivered = true;
                }
                if !delivered {
                    return Err(Error::Handler(format!("topic {topic} has no subscription")));
                }
                queue.published.fetch_add(1, Ordering::SeqCst);
                // queue/src/functions.rs PublishAck is null, not {ok:true}.
                Ok(Value::Null)
            })
            .description("Local queue acceptance with explicit dispatch")
            .request_format(json!({"type":"object"}))
            .response_format(json!({"type":"null"})),
        );
    }
    /// Messages waiting in every subscription, consumed or not.
    pub fn len(&self) -> usize {
        self.subscriptions
            .lock()
            .unwrap()
            .values()
            .map(|s| s.backlog.len())
            .sum()
    }
    /// Delivers the backlog of every subscription that has an observer, one
    /// message at a time, as a call of `queue::on-message` for that
    /// observer; a subscription nobody consumes keeps its backlog.
    pub async fn drain(&self, iii: &IIIClient) -> Result<(), Error> {
        for _ in 0..100 {
            let next = {
                let mut subscriptions = self.subscriptions.lock().unwrap();
                let observers = self.observers.values();
                subscriptions.iter_mut().find_map(|(key, subscription)| {
                    let obs = observers.iter().find(|obs| {
                        let spec = SubscriberSpec::of(obs);
                        spec.queue == subscription.topic && spec.subscription(obs) == *key
                    })?;
                    let payload = subscription.backlog.pop_front()?;
                    Some((key.clone(), obs.clone(), payload))
                })
            };
            let Some((key, obs, payload)) = next else {
                return Ok(());
            };
            let spec = SubscriberSpec::of(&obs);
            assert_eq!(spec.max_retries, Some(5));
            assert_eq!(spec.backoff_ms, Some(1000));
            assert_eq!(
                spec.group.as_deref(),
                Some(github::webhooks::WEBHOOK_CONSUMER_GROUP)
            );
            let message = CallRequest::new(ON_MESSAGE, payload.clone())
                .metadata(json!({ "subscriber": obs.id() }))
                .timeout(Duration::from_secs(10));
            if let Err(error) = iii.call(message).await {
                // A failed real handler is NOT ACKed. Explicit next drain retries it.
                if let Some(subscription) = self.subscriptions.lock().unwrap().get_mut(&key) {
                    subscription.backlog.push_front(payload);
                }
                return Err(error);
            }
        }
        panic!("queue dispatch exceeded bounded test budget");
    }
}

#[derive(Clone)]
pub struct Tunnel {
    pub observers: Observers,
    pub cron: Observers,
    snapshot: Arc<Mutex<Value>>,
    leases: Arc<Mutex<HashMap<String, Value>>>,
    pub releases: Arc<Mutex<Vec<String>>>,
}
impl Default for Tunnel {
    fn default() -> Self {
        Self {
            observers: Observers::default(),
            cron: Observers::default(),
            snapshot: Arc::new(Mutex::new(json!({
                "tunnel_id":"webhooks","status":"starting","public_url":null,"generation":"starting-1","error":null
            }))),
            leases: Default::default(),
            releases: Default::default(),
        }
    }
}
impl Tunnel {
    pub fn register(&self, iii: &IIIClient) {
        // Each call is one tunnel snapshot, delivered by iii (default
        // delivery). quick-tunnel::on-changed takes no args: observers filter
        // on input.tunnel_id.
        self.observers
            .host(iii, ON_CHANGED, "Local controlled tunnel lifecycle", None);
        self.cron.host(
            iii,
            "cron::on-tick",
            "Recorded only; no wall-clock scheduler",
            Some(json!({"type":"object"})),
        );
        let this = self.clone();
        iii.register_function("quick-tunnel::acquire", RegisterFunction::new(move |input: Value| {
            let consumer = input["consumer_id"].as_str().ok_or_else(|| Error::Handler("consumer required".into()))?;
            assert_eq!(input["tunnel_id"], "webhooks");
            let expiry = chrono::DateTime::parse_from_rfc3339(input["expires_at"].as_str().unwrap()).unwrap();
            assert!(expiry > chrono::Utc::now() && expiry < chrono::Utc::now() + chrono::Duration::days(30));
            let mut leases = this.leases.lock().unwrap();
            let lease = leases.entry(consumer.into()).or_insert_with(|| json!({
                "lease_id":uuid::Uuid::new_v4().to_string(),"consumer_id":consumer,"tunnel_id":"webhooks","expires_at":input["expires_at"]
            }));
            let mut result = this.snapshot.lock().unwrap().clone();
            result["lease_id"] = lease["lease_id"].clone();
            Ok::<Value, Error>(result)
        }).description("Local idempotent tunnel lease").request_format(json!({"type":"object"})).response_format(json!({"type":"object"})));
        let this = self.clone();
        iii.register_function(
            "quick-tunnel::status",
            RegisterFunction::new(move |_input: Value| {
                let mut result = this.snapshot.lock().unwrap().clone();
                result["leases"] = json!(this.leases.lock().unwrap().values().collect::<Vec<_>>());
                Ok::<Value, Error>(result)
            })
            .description("Local authoritative tunnel snapshot")
            .request_format(json!({"type":"object"}))
            .response_format(json!({"type":"object"})),
        );
        let this = self.clone();
        iii.register_function(
            "quick-tunnel::release",
            RegisterFunction::new(move |input: Value| {
                let id = input["lease_id"]
                    .as_str()
                    .ok_or_else(|| Error::Handler("lease_id required".into()))?;
                let mut leases = this.leases.lock().unwrap();
                let before = leases.len();
                leases.retain(|_, lease| lease["lease_id"] != id);
                let released = leases.len() != before;
                if released {
                    this.releases.lock().unwrap().push(id.into());
                }
                Ok::<Value, Error>(json!({"released":released}))
            })
            .description("Local tunnel release")
            .request_format(json!({"type":"object"}))
            .response_format(json!({"type":"object"})),
        );
    }
    pub fn lease_count(&self) -> usize {
        self.leases.lock().unwrap().len()
    }
    /// The tunnel is ready: calls quick-tunnel::on-changed with the snapshot,
    /// which iii delivers to the observers, and returns the snapshot. The
    /// call returns before the observers' targets answer.
    pub async fn ready(&self, iii: &IIIClient, url: &str, generation: &str) -> Value {
        let value = json!({"tunnel_id":"webhooks","status":"ready","public_url":url,"generation":generation,"error":null});
        // A ready event and the status read expose the SAME authoritative generation.
        *self.snapshot.lock().unwrap() = value.clone();
        let observers = self.observers.values();
        assert_eq!(observers.len(), 1);
        let other = json!({"tunnel_id":"other","status":"ready"});
        for obs in observers {
            // The observer selects its tunnel with a filter on input.tunnel_id.
            assert!(obs
                .args_value()
                .as_object()
                .is_none_or(|args| args.is_empty()));
            assert!(!obs.matches(&event_call(other.clone())));
            assert!(obs.matches(&event_call(value.clone())));
        }
        call(iii, ON_CHANGED, value.clone()).await;
        value
    }
}
/// A call of a source whose handler returned `event`.
fn event_call(event: Value) -> Call {
    Call {
        output: Some(event.clone()),
        input: event,
        ..Call::default()
    }
}
/// A call through `obs`, made only in a deliver hook: its target receives
/// `payload`.
fn through(obs: &Observer, payload: Value) -> iii_sdk::CallRequestWithMetadata {
    CallRequest::observer(obs, payload).timeout(Duration::from_secs(10))
}
pub fn request(id: &str, payload: Value) -> CallRequest {
    CallRequest {
        function_id: id.into(),
        payload,
        action: None,
        timeout_ms: Some(10_000),
    }
}
pub async fn call(iii: &IIIClient, id: &str, payload: Value) -> Value {
    iii.call(request(id, payload))
        .await
        .unwrap_or_else(|e| panic!("{id}: {e}"))
}
