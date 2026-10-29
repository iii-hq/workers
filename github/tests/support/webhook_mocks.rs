//! Contract mocks, not the production queue adapter/cloudflared process.
//! Queue ACK means accepted; dispatch is explicit so assertions can inspect the
//! real SQLite inbox before the real registered process handler consumes it.
//! Each mock source is a private function whose hooks keep its observers; the
//! mock delivers by calling through them, as the real queue worker does.
use iii_sdk::{Call, CallRequest, Error, IIIClient, ObserveRejection, Observer, RegisterFunction};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

pub const PROVIDER: &str = "integration-provider";
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
}
#[derive(Clone, Default)]
pub struct Queue {
    pub observers: Observers,
    pending: Arc<Mutex<VecDeque<(String, Value)>>>,
    pub fail: Arc<AtomicBool>,
    pub failed_publications: Arc<AtomicUsize>,
    pub published: Arc<AtomicUsize>,
}
impl Queue {
    pub fn register(&self, iii: &IIIClient) {
        self.observers.host(
            iii,
            "queue::on-message",
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
                queue
                    .pending
                    .lock()
                    .unwrap()
                    .push_back((topic.into(), data.clone()));
                queue.published.fetch_add(1, Ordering::SeqCst);
                // queue/src/functions.rs PublishAck is null, not {ok:true}.
                Ok(Value::Null)
            })
            .description("Local queue acceptance with explicit dispatch")
            .request_format(json!({"type":"object"}))
            .response_format(json!({"type":"null"})),
        );
    }
    pub fn len(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
    pub async fn drain(&self, iii: &IIIClient) -> Result<(), Error> {
        for _ in 0..100 {
            let Some((topic, payload)) = self.pending.lock().unwrap().pop_front() else {
                return Ok(());
            };
            let mut matched = false;
            for obs in self.observers.values() {
                let spec: SubscriberSpec =
                    serde_json::from_value(obs.args_value().clone()).unwrap();
                if spec.queue != topic {
                    continue;
                }
                matched = true;
                assert_eq!(spec.max_retries, Some(5));
                assert_eq!(spec.backoff_ms, Some(1000));
                if let Err(error) = iii.call(through(&obs, payload.clone())).await {
                    // A failed real handler is NOT ACKed. Explicit next drain retries it.
                    self.pending.lock().unwrap().push_front((topic, payload));
                    return Err(error);
                }
            }
            assert!(matched, "publication has no observer");
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
        // quick-tunnel::on-changed takes no args: observers filter on input.tunnel_id.
        self.observers.host(
            iii,
            "quick-tunnel::on-changed",
            "Local controlled tunnel lifecycle",
            None,
        );
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
    pub async fn ready(&self, iii: &IIIClient, url: &str, generation: &str) {
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
            iii.call(through(&obs, value.clone())).await.unwrap();
        }
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
/// A call through `obs`: its target receives `payload`.
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
