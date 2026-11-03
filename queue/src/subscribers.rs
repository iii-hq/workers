//! The `queue::on-message` subscribers, the [`Invoker`] the adapters
//! deliver through, and the engine change feed held deliveries wait on.
//!
//! Every subscriber is a `queue::on-message` observer ([`crate::on_message`]).
//! [`Subscribers`] turns one into
//! [`crate::adapter::QueueAdapter::subscribe`]/`unsubscribe` calls against
//! the (possibly hot-swapped) [`SwappableAdapter`]. All delivery, retry, and
//! DLQ behavior lives in the adapter (e.g.
//! [`crate::adapters::builtin::BuiltinAdapter`]): its consumer loop hands each
//! message to [`Subscriber::deliver`], one call of `queue::on-message` for
//! that observer, and the result acks the message or fails it.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, Deliver, IIIClient, RegisterFunction, Source};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{watch, Mutex};

use crate::adapter::{QueueAdapter, SwappableAdapter};
use crate::boot::ConfigCell;
use crate::config::DEFAULT_SUBSCRIBER_RETENTION_MS;
use crate::on_message::{ON_MESSAGE_FN, SUBSCRIBER_METADATA_KEY};
use crate::subscriber_config::SubscriberQueueConfig;

/// Queue-delivered functions may run for many minutes (e.g. streaming an LLM
/// response). The iii SDK defaults an omitted timeout to 30 seconds, which
/// lets the adapter retry an in-flight delivery while the original invocation
/// is still running. 30 minutes covers the longest intended job budget.
pub(crate) const FUNCTION_QUEUE_INVOCATION_TIMEOUT_MS: u64 = 30 * 60 * 1_000;

/// How long one engine-epoch sample may take.
const ENGINE_EPOCH_PROBE_TIMEOUT_MS: u64 = 3_000;
/// Internal function that observes the engine's registry changes; see
/// [`observe_engine_events`].
pub const ENGINE_CHANGE_FN_ID: &str = "queue::on-engine-change";
/// Called by the engine when functions are registered/unregistered.
const FUNCTIONS_CHANGED_FN: &str = "engine::on-functions-changed";
/// Called by the engine when a worker connects, disconnects, or announces its
/// metadata (`engine::workers::register`) — the latter only after that
/// worker's buffered function registrations and observers have been applied.
const WORKERS_CHANGED_FN: &str = "engine::on-workers-changed";
/// Starts a group's subscription key; escaped inside a group name.
const GROUP_KEY_PREFIX: char = '@';
const SUBSCRIPTION_KEY_ESCAPE: char = '\\';
/// Starts the subscription key of an observer outside a group.
const OBSERVER_KEY_PREFIX: &str = "observer:";

/// The engine's boot identity: the earliest `connected_at_ms` among its
/// in-process (`runtime == "engine"`) workers, which attach once at engine
/// startup. A restarted engine reports a new epoch — the signal that every
/// invocation in flight across the restart lost its result routing. `None`
/// when the engine cannot be reached (outage in progress) or the response
/// shape is unrecognized.
async fn engine_epoch_ms(iii: &IIIClient) -> Option<u64> {
    let response = iii
        .call(CallRequest {
            function_id: "engine::workers::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(ENGINE_EPOCH_PROBE_TIMEOUT_MS),
        })
        .await
        .ok()?;
    response
        .get("workers")?
        .as_array()?
        .iter()
        .filter(|worker| worker.get("runtime").and_then(Value::as_str) == Some("engine"))
        .filter_map(|worker| worker.get("connected_at_ms").and_then(Value::as_u64))
        .min()
}

/// Engine registry change feed: a generation counter bumped every time the
/// engine reports that functions were registered/unregistered
/// (`engine::on-functions-changed`) or that a worker connected, disconnected,
/// or (re)announced itself (`engine::on-workers-changed`) — which includes
/// this worker's own reconnect to a restarted engine, since the SDK replays
/// its observers before announcing.
///
/// Work held until the engine changes (a target function appearing, an
/// engine restart under an in-flight invocation) re-checks once per change
/// instead of re-checking on a timer. [`EngineEvents::default`] is an
/// unbound feed that never fires; [`observe_engine_events`] wires one to the
/// engine.
#[derive(Clone)]
pub struct EngineEvents {
    tx: Arc<watch::Sender<u64>>,
}

impl Default for EngineEvents {
    fn default() -> Self {
        Self {
            tx: Arc::new(watch::Sender::new(0)),
        }
    }
}

impl EngineEvents {
    /// Record one engine change, waking every subscriber.
    pub fn notify(&self) {
        self.tx
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }

    /// Start observing changes. Take the subscription BEFORE checking the
    /// condition being waited on: every change after this call wakes
    /// [`EngineChanges::changed`], so one landing between the check and the
    /// wait is never lost.
    pub fn subscribe(&self) -> EngineChanges {
        EngineChanges {
            rx: Some(self.tx.subscribe()),
        }
    }
}

/// One subscriber's view of an [`EngineEvents`] feed.
pub struct EngineChanges {
    rx: Option<watch::Receiver<u64>>,
}

impl EngineChanges {
    /// A feed that never fires, for invokers with nothing to wait on.
    pub fn never() -> Self {
        Self { rx: None }
    }

    /// Resolve once at least one change has happened since the subscription
    /// was taken or since the previous `changed` returned (bursts coalesce).
    /// Never resolves on an unbound or closed feed.
    pub async fn changed(&mut self) {
        if let Some(rx) = self.rx.as_mut() {
            if rx.changed().await.is_ok() {
                return;
            }
        }
        std::future::pending::<()>().await
    }
}

/// The event of `engine::on-functions-changed` / `engine::on-workers-changed`,
/// delivered as output. Only the advisory event tag is read; the larger
/// fields (`functions`, `worker_id`) are ignored because a change is only a
/// cue to re-check.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct EngineChangeEvent {
    #[serde(default)]
    pub event: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EngineChangeAck {
    pub ok: bool,
}

/// Register the internal [`ENGINE_CHANGE_FN_ID`] handler and observe
/// `engine::on-functions-changed` and `engine::on-workers-changed` with it,
/// returning the feed it drives. Call once per client. The observers stay
/// `pending` until the engine hosts the sources; held deliveries then wait
/// for their own invocation timeout.
pub fn observe_engine_events(iii: &IIIClient) -> EngineEvents {
    let events = EngineEvents::default();
    let handler_events = events.clone();
    iii.register_function(
        ENGINE_CHANGE_FN_ID,
        RegisterFunction::new_async(move |event: EngineChangeEvent| {
            let events = handler_events.clone();
            async move {
                tracing::debug!(event = ?event.event, "engine registry changed");
                events.notify();
                Ok::<_, Error>(EngineChangeAck { ok: true })
            }
        })
        .description(
            "Internal: wake queue deliveries held on an engine change (a target function \
             registering, an engine restart) when functions or workers come and go.",
        )
        .metadata(json!({ "internal": true })),
    );
    for source in [FUNCTIONS_CHANGED_FN, WORKERS_CHANGED_FN] {
        // The observers live as long as the worker's connection; dropping
        // the handles does not dispose them.
        iii.observe(
            Source::function(source).deliver(Deliver::Output),
            ENGINE_CHANGE_FN_ID,
        );
    }
    events
}

/// Resolve once `probe` reports an epoch other than `baseline`, re-probing
/// once per engine change. An unreadable epoch (`None`, outage in progress)
/// never trips by itself; the next change decides. `changes` must have been
/// subscribed before `baseline` was sampled.
pub async fn wait_for_epoch_change<P, F>(baseline: u64, mut changes: EngineChanges, mut probe: P)
where
    P: FnMut() -> F,
    F: Future<Output = Option<u64>>,
{
    loop {
        changes.changed().await;
        if probe().await.is_some_and(|epoch| epoch != baseline) {
            return;
        }
    }
}

#[async_trait]
pub trait Invoker: Send + Sync + 'static {
    async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String>;

    async fn call_with_timeout(
        &self,
        function_id: &str,
        payload: Value,
        _timeout_ms: u64,
        namespace: &str,
    ) -> Result<Option<Value>, String> {
        let _ = namespace;
        self.call(function_id, payload).await
    }

    /// Hand one message to the subscriber `subscriber` (a `queue::on-message`
    /// observer id): one call of `queue::on-message` naming it, whose deliver
    /// hook calls through that observer. `Ok` acks the message; `Err` fails
    /// it (retry with backoff, then the DLQ). The default calls
    /// [`Self::call`] with the subscriber id, which suits test invokers; the
    /// iii-backed invoker overrides it.
    async fn deliver(&self, subscriber: &str, payload: Value) -> Result<Option<Value>, String> {
        self.call(subscriber, payload).await
    }

    /// Whether the target is currently registered with the engine.
    ///
    /// Adapters used in unit tests and embedded deployments can keep the
    /// optimistic default. The real iii-backed invoker overrides this so a
    /// durable job restored before its worker boots is held instead of
    /// burning delivery retries on a transient `FUNCTION_NOT_FOUND` error.
    async fn function_available(
        &self,
        _function_id: &str,
        namespace: &str,
    ) -> Result<bool, String> {
        let _ = namespace;
        Ok(true)
    }

    /// Engine registry change feed (see [`EngineEvents`]). Waiters subscribe
    /// before checking and re-check once per change. The default never
    /// fires, which suits invokers whose targets are always available and
    /// whose connection never restarts.
    fn engine_changes(&self) -> EngineChanges {
        EngineChanges::never()
    }

    /// Whether a function-queue job whose target is not registered goes back to the broker
    /// instead of being held: see [`IiiInvoker::with_worker_namespace`].
    fn hands_back_unavailable(&self) -> bool {
        false
    }

    /// Capture the engine epoch immediately before a restart-sensitive
    /// invocation. `None` disables restart watching for this invocation.
    async fn connection_epoch(&self) -> Result<Option<u64>, String> {
        Ok(None)
    }

    /// Resolve when the engine moves away from the captured epoch. `changes`
    /// is the [`Self::engine_changes`] subscription taken before `baseline`
    /// was sampled.
    async fn connection_lost_since(&self, _baseline: u64, _changes: EngineChanges) {
        std::future::pending::<()>().await
    }
}

#[derive(Clone)]
pub struct IiiInvoker {
    iii: Arc<IIIClient>,
    events: EngineEvents,
    worker_namespace: Option<String>,
}

impl IiiInvoker {
    /// An invoker with an unbound [`EngineEvents`] feed: enough for plain
    /// deliveries, but a delivery held for an unregistered target (or an
    /// engine restart) is only re-checked by [`Self::with_engine_events`].
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            iii,
            events: EngineEvents::default(),
            worker_namespace: None,
        }
    }

    /// Attach the feed returned by [`observe_engine_events`].
    pub fn with_engine_events(mut self, events: EngineEvents) -> Self {
        self.events = events;
        self
    }

    /// [`RunJobsIn::Worker`](crate::config::RunJobsIn::Worker): a function-queue job runs in this
    /// worker's own namespace, whichever namespace enqueued it, and one whose function is not
    /// registered here goes back to the broker for another worker.
    pub fn with_worker_namespace(mut self, namespace: String) -> Self {
        self.worker_namespace = Some(namespace);
        self
    }

    fn job_namespace<'a>(&'a self, enqueued_in: &'a str) -> &'a str {
        self.worker_namespace.as_deref().unwrap_or(enqueued_in)
    }
}

#[async_trait]
impl Invoker for IiiInvoker {
    async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String> {
        self.iii
            .call(CallRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(FUNCTION_QUEUE_INVOCATION_TIMEOUT_MS),
            })
            .await
            .map(Some)
            .map_err(|e| e.to_string())
    }

    async fn call_with_timeout(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: u64,
        namespace: &str,
    ) -> Result<Option<Value>, String> {
        self.iii
            .call(
                CallRequest {
                    function_id: function_id.to_string(),
                    payload,
                    action: None,
                    timeout_ms: Some(timeout_ms),
                }
                .namespace(self.job_namespace(namespace)),
            )
            .await
            .map(Some)
            .map_err(|e| e.to_string())
    }

    async fn deliver(&self, subscriber: &str, payload: Value) -> Result<Option<Value>, String> {
        // A local call of this worker's own private function: its deliver
        // hook calls through the observer, and the target's result is this
        // call's result.
        let request = CallRequest {
            function_id: ON_MESSAGE_FN.to_string(),
            payload,
            action: None,
            timeout_ms: Some(FUNCTION_QUEUE_INVOCATION_TIMEOUT_MS),
        }
        .metadata(json!({ SUBSCRIBER_METADATA_KEY: subscriber }));
        self.iii
            .call(request)
            .await
            .map(Some)
            .map_err(|e| e.to_string())
    }

    async fn function_available(&self, function_id: &str, namespace: &str) -> Result<bool, String> {
        match self
            .iii
            .call(CallRequest {
                function_id: "engine::functions::info".to_string(),
                payload: json!({
                    "function_id": function_id,
                    "namespace": self.job_namespace(namespace),
                }),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await
        {
            Ok(value) => Ok(!value.is_null() && value.get("error").is_none()),
            Err(error) => {
                let message = error.to_string();
                if message.to_ascii_uppercase().contains("NOT_FOUND") {
                    Ok(false)
                } else {
                    Err(message)
                }
            }
        }
    }

    fn engine_changes(&self) -> EngineChanges {
        self.events.subscribe()
    }

    fn hands_back_unavailable(&self) -> bool {
        self.worker_namespace.is_some()
    }

    async fn connection_epoch(&self) -> Result<Option<u64>, String> {
        Ok(engine_epoch_ms(&self.iii).await)
    }

    async fn connection_lost_since(&self, baseline: u64, changes: EngineChanges) {
        // Neither the SDK connection state nor plain liveness probes can see
        // a fast restart: the reconnect loop reports `Connected` through its
        // silent 2s retry sleep, and outbound messages buffered during the
        // outage are answered by the NEW engine as if nothing happened
        // (both verified against a SIGKILLed-and-respawned engine), and the
        // SDK exposes no reconnect event. The engine's boot epoch is the
        // reliable signal, sampled once per engine change: after a restart
        // the SDK replays this worker's registrations — including its
        // `engine::on-workers-changed` observer — and then announces itself,
        // so the restarted engine calls that source and our observer's
        // target, and the sample taken in response reveals the new epoch.
        wait_for_epoch_change(baseline, changes, || engine_epoch_ms(&self.iii)).await
    }
}

/// The `queue::on-message` observer an adapter subscription delivers into.
/// Each message is one call of `queue::on-message` naming it
/// ([`Invoker::deliver`]); the observer's target's result acks the message,
/// or fails it so it is retried and then dead-lettered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscriber {
    id: String,
}

impl Subscriber {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }

    /// The observer id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Hand one message to the subscriber. `Ok` acks it; `Err` fails it
    /// (retry with backoff, then the DLQ).
    pub async fn deliver(
        &self,
        invoker: &dyn Invoker,
        payload: Value,
    ) -> Result<Option<Value>, String> {
        invoker.deliver(&self.id, payload).await
    }
}

/// A `queue::on-message` observer's args. Any other field is `INVALID_ARGS`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubscriberSpec {
    /// Queue/topic name to consume.
    #[serde(alias = "topic")]
    pub queue: String,
    #[serde(default, alias = "maxRetries", skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    #[serde(
        default,
        alias = "backoffDelayMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub backoff_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_config: Option<SubscriberQueueConfig>,
    /// Consumer group: the observers of this topic with the same group share
    /// one durable queue, so each message is handled once by the group,
    /// whatever their target, namespace or worker. Without it a subscription
    /// is the observer's own, keyed by its id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RegisteredSubscriber {
    /// The `queue::on-message` observer's id.
    pub id: String,
    pub spec: SubscriberSpec,
}

impl RegisteredSubscriber {
    /// The durable identity the adapter keys this subscription's queue by.
    ///
    /// An observer outside a group is keyed by `observer:` and its id, which
    /// its SDK keeps across reconnects and which a consumer pins with the
    /// `id` observe option: the same id finds the same queue and its backlog
    /// after the observing worker restarts.
    ///
    /// A [`SubscriberSpec::group`] is the identity instead: `@` and the group,
    /// whatever the observer. An observer's key never starts with `@`, so a
    /// group never meets it.
    fn subscription_key(&self) -> String {
        match self.spec.group.as_deref().filter(|g| !g.is_empty()) {
            Some(group) => format!(
                "{GROUP_KEY_PREFIX}{}",
                escape_subscription_key_segment(group)
            ),
            None => format!("{OBSERVER_KEY_PREFIX}{}", self.id),
        }
    }

    /// Who this registration's subscription delivers into.
    pub fn subscriber(&self) -> Subscriber {
        Subscriber::new(self.id.clone())
    }
}

/// Escape the group prefix and the escape character inside a group name, so
/// the single unescaped `@` that [`RegisteredSubscriber::subscription_key`]
/// puts in front of a group is unambiguous.
fn escape_subscription_key_segment(segment: &str) -> String {
    segment
        .replace(
            SUBSCRIPTION_KEY_ESCAPE,
            &format!("{SUBSCRIPTION_KEY_ESCAPE}{SUBSCRIPTION_KEY_ESCAPE}"),
        )
        .replace(
            GROUP_KEY_PREFIX,
            &format!("{SUBSCRIPTION_KEY_ESCAPE}{GROUP_KEY_PREFIX}"),
        )
}

/// Merge the spec's top-level `max_retries`/`backoff_ms` into its
/// `queue_config`. Precedence: explicit top-level field wins, then whatever
/// `queue_config` already carries, then the adapter's own defaults (3
/// retries / 1000ms backoff) when both are absent -- the adapter applies
/// those defaults itself when the returned fields are `None`.
fn merged_queue_config(spec: &SubscriberSpec) -> SubscriberQueueConfig {
    let mut config = spec.queue_config.clone().unwrap_or_default();
    if let Some(max_retries) = spec.max_retries {
        config.max_retries = Some(max_retries);
    }
    if let Some(backoff_ms) = spec.backoff_ms {
        config.backoff_delay_ms = Some(backoff_ms);
    }
    config
}

/// Why [`Subscribers::register`] refused a subscriber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// The args fail validation (no topic).
    InvalidConfig(String),
    /// A subscriber with this observer id is already tracked.
    AlreadyRegistered(String),
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(message) | Self::AlreadyRegistered(message) => f.write_str(message),
        }
    }
}

/// The subscribers: one per `queue::on-message` observer, by observer id.
#[derive(Clone)]
pub struct Subscribers {
    adapter: Arc<SwappableAdapter>,
    registrations: Arc<Mutex<HashMap<String, RegisteredSubscriber>>>,
    /// The registration each live adapter subscription delivers into, by
    /// `(queue, key)`. Locked only under `registrations`.
    served_by: Arc<Mutex<HashMap<(String, String), String>>>,
    /// The live configuration, read for `subscriber_retention_ms` when an
    /// observer leaves. Without it the retention is
    /// [`DEFAULT_SUBSCRIBER_RETENTION_MS`].
    config: Option<ConfigCell>,
}

impl Subscribers {
    pub fn new(adapter: Arc<SwappableAdapter>) -> Self {
        Self {
            adapter,
            registrations: Arc::new(Mutex::new(HashMap::new())),
            served_by: Arc::new(Mutex::new(HashMap::new())),
            config: None,
        }
    }

    /// Read `subscriber_retention_ms` from `config` each time an observer
    /// leaves, so a configuration change applies to the next one.
    pub fn with_config(mut self, config: ConfigCell) -> Self {
        self.config = Some(config);
        self
    }

    /// How long the queue of a subscription no observer serves keeps
    /// receiving its topic.
    async fn retention(&self) -> Duration {
        match &self.config {
            Some(config) => config.read().await.subscriber_retention(),
            None => Duration::from_millis(DEFAULT_SUBSCRIBER_RETENTION_MS),
        }
    }

    /// Open the adapter subscription for `key`, delivering into `registration`.
    async fn subscribe_for(&self, registration: &RegisteredSubscriber, key: &str) {
        let queue_config = merged_queue_config(&registration.spec);
        self.adapter
            .subscribe(
                &registration.spec.queue,
                key,
                registration.subscriber(),
                Some(queue_config),
            )
            .await;
        self.served_by.lock().await.insert(
            (registration.spec.queue.clone(), key.to_string()),
            registration.id.clone(),
        );
    }

    /// Snapshot of every currently registered subscriber -- consumed by the
    /// configuration hot-swap path to re-subscribe against the newly built
    /// adapter after a transport swap.
    pub async fn registrations(&self) -> Vec<RegisteredSubscriber> {
        self.registrations.lock().await.values().cloned().collect()
    }

    /// Clears every tracked registration and tears down the current
    /// adapter's active consumers.
    pub async fn shutdown(&self) {
        self.registrations.lock().await.clear();
        self.adapter.shutdown().await;
    }

    /// Re-subscribe every currently tracked registration directly against
    /// the adapter, bypassing [`Self::register`]'s duplicate-id check and
    /// leaving the registrations map untouched. Used by the configuration
    /// hot-swap path (`crate::configuration::swap_adapter`) *after*
    /// [`SwappableAdapter::replace`] has pointed `self.adapter` at the newly
    /// built transport, so these `subscribe` calls attach consumers to the
    /// new adapter rather than the one being retired.
    pub async fn resubscribe_all(&self) {
        let mut seen = HashSet::new();
        for registration in self.registrations().await {
            let key = registration.subscription_key();
            if !seen.insert((registration.spec.queue.clone(), key.clone())) {
                continue; // a group's members share one subscription
            }
            self.subscribe_for(&registration, &key).await;
        }
    }

    /// Track `registration` and attach it to the adapter. Its consumer starts
    /// here, before the SDK acks the observer, so no message published after
    /// the observer is active is missed. A subscription whose queue is within
    /// its retention (see [`Self::unregister`]) is consumed again from
    /// everything it holds.
    pub async fn register(&self, registration: RegisteredSubscriber) -> Result<(), RegisterError> {
        if registration.spec.queue.trim().is_empty() {
            return Err(RegisterError::InvalidConfig(format!(
                "topic is required for a {ON_MESSAGE_FN} observer"
            )));
        }

        let mut registrations = self.registrations.lock().await;
        if registrations.contains_key(&registration.id) {
            return Err(RegisterError::AlreadyRegistered(format!(
                "{ON_MESSAGE_FN} observer '{}' is already registered",
                registration.id
            )));
        }

        // The members of a group share its one live subscription instead of
        // stacking consumers that would each deliver every message again.
        let key = registration.subscription_key();
        let already_live = registrations
            .values()
            .any(|r| r.spec.queue == registration.spec.queue && r.subscription_key() == key);
        if !already_live {
            self.subscribe_for(&registration, &key).await;
        }

        registrations.insert(registration.id.clone(), registration);
        Ok(())
    }

    /// Unregister the observer `id`. When no other observer serves its
    /// subscription, the subscription's consumer stops at once and its queue
    /// keeps receiving the topic for `subscriber_retention_ms`: an observer
    /// with the same subscription key within that time consumes everything
    /// it received. After it, the queue keeps only what it holds, for a later
    /// observer with the same id. A group's subscription that delivered into
    /// the observer re-opens toward a member that stays.
    ///
    /// The registrations lock is deliberately held across `unsubscribe`:
    /// releasing it first opens a window where a same-key register sees no
    /// registration, calls subscribe while the adapter still holds the old
    /// entry, no-ops on the adapter's duplicate check, and ends up
    /// registered with no live consumer. Adapter unsubscribes are prompt
    /// (they detach their drain rather than await it), so holding the lock
    /// costs a broker round-trip, not an invocation.
    pub async fn unregister(&self, id: &str) {
        let mut registrations = self.registrations.lock().await;
        let Some(registration) = registrations.remove(id) else {
            return;
        };
        let key = registration.subscription_key();
        let served = (registration.spec.queue.clone(), key.clone());
        let next = registrations
            .values()
            .find(|r| r.spec.queue == registration.spec.queue && r.subscription_key() == key)
            .cloned();
        let served_this = self.served_by.lock().await.get(&served) == Some(&registration.id);
        // Also on a re-open: the queue keeps receiving the topic while the
        // subscription moves to the member that stays.
        let retain = self.retention().await;
        match next {
            None => {
                self.adapter
                    .unsubscribe(&registration.spec.queue, &key, retain)
                    .await;
                self.served_by.lock().await.remove(&served);
            }
            // The group's subscription delivered into the observer that
            // left: re-open it toward a member that stays.
            Some(next) if served_this => {
                self.adapter
                    .unsubscribe(&registration.spec.queue, &key, retain)
                    .await;
                self.subscribe_for(&next, &key).await;
            }
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::TopicInfo;
    use crate::store::TopicStats;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    // A change recorded after subscribing but before waiting is not lost; an
    // unbound feed never fires.
    #[tokio::test(start_paused = true)]
    async fn engine_change_before_the_wait_is_not_lost() {
        let events = EngineEvents::default();
        let mut changes = events.subscribe();
        events.notify();
        events.notify();
        tokio::time::timeout(Duration::from_secs(1), changes.changed())
            .await
            .expect("a change taken after subscribing must wake the waiter");
        assert!(
            tokio::time::timeout(Duration::from_secs(3600), changes.changed())
                .await
                .is_err(),
            "bursts coalesce: two notifies before one wait are one change"
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(3600), EngineChanges::never().changed())
                .await
                .is_err()
        );
    }

    // The restart watch samples the engine epoch once per engine change and
    // never on a timer: an hour of virtual time with no change costs no
    // probe. An unreadable epoch (outage) or an unchanged one keeps waiting.
    #[tokio::test(start_paused = true)]
    async fn epoch_is_probed_once_per_engine_change_never_on_a_timer() {
        let events = EngineEvents::default();
        let changes = events.subscribe();
        let answers = Arc::new(StdMutex::new(VecDeque::from([None, Some(1), Some(2)])));
        let probes = Arc::new(AtomicUsize::new(0));
        let watcher = tokio::spawn({
            let answers = answers.clone();
            let probes = probes.clone();
            async move {
                wait_for_epoch_change(1, changes, || {
                    probes.fetch_add(1, Ordering::SeqCst);
                    let answer = answers.lock().unwrap().pop_front().flatten();
                    async move { answer }
                })
                .await
            }
        });

        let idle_hour = || tokio::time::sleep(Duration::from_secs(3600));
        idle_hour().await;
        assert_eq!(probes.load(Ordering::SeqCst), 0);
        for expected_probes in [1, 2] {
            events.notify();
            idle_hour().await;
            assert_eq!(probes.load(Ordering::SeqCst), expected_probes);
            assert!(
                !watcher.is_finished(),
                "outage or same epoch must keep waiting"
            );
        }
        events.notify();
        tokio::time::timeout(Duration::from_secs(1), watcher)
            .await
            .expect("a changed epoch must resolve the watch")
            .unwrap();
        assert_eq!(probes.load(Ordering::SeqCst), 3);
    }

    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        Subscribe {
            topic: String,
            id: String,
            subscriber: String,
            queue_config: Option<SubscriberQueueConfig>,
        },
        Unsubscribe {
            topic: String,
            id: String,
            retain: Duration,
        },
    }

    #[derive(Default)]
    struct MockAdapter {
        calls: StdMutex<Vec<Call>>,
    }

    impl MockAdapter {
        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl QueueAdapter for MockAdapter {
        async fn enqueue(
            &self,
            _topic: &str,
            _data: Value,
            _traceparent: Option<String>,
            _baggage: Option<String>,
        ) {
        }

        async fn subscribe(
            &self,
            topic: &str,
            id: &str,
            subscriber: Subscriber,
            queue_config: Option<SubscriberQueueConfig>,
        ) {
            self.calls.lock().unwrap().push(Call::Subscribe {
                topic: topic.to_string(),
                id: id.to_string(),
                subscriber: subscriber.id().to_string(),
                queue_config,
            });
        }

        async fn unsubscribe(&self, topic: &str, id: &str, retain: Duration) {
            self.calls.lock().unwrap().push(Call::Unsubscribe {
                topic: topic.to_string(),
                id: id.to_string(),
                retain,
            });
        }

        async fn redrive_dlq(&self, _topic: &str) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn redrive_dlq_message(
            &self,
            _topic: &str,
            _message_id: &str,
        ) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn discard_dlq_message(
            &self,
            _topic: &str,
            _message_id: &str,
        ) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn dlq_count(&self, _topic: &str) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn list_topics(&self) -> anyhow::Result<Vec<TopicInfo>> {
            Ok(vec![])
        }

        async fn topic_stats(&self, _topic: &str) -> anyhow::Result<TopicStats> {
            Ok(TopicStats::default())
        }

        async fn shutdown(&self) {}
    }

    fn registration(id: &str, args: Value) -> RegisteredSubscriber {
        RegisteredSubscriber {
            id: id.to_string(),
            spec: serde_json::from_value(args).unwrap(),
        }
    }

    fn subscribers_with_mock() -> (Subscribers, Arc<MockAdapter>) {
        let mock = Arc::new(MockAdapter::default());
        let adapter: Arc<dyn QueueAdapter> = mock.clone();
        let subscribers = Subscribers::new(Arc::new(SwappableAdapter::new(adapter, "mock")));
        (subscribers, mock)
    }

    #[tokio::test]
    async fn register_calls_adapter_subscribe_with_merged_config() {
        let (subscribers, mock) = subscribers_with_mock();

        subscribers
            .register(registration(
                "obs-1",
                json!({"topic": "demo", "max_retries": 2, "backoff_ms": 1}),
            ))
            .await
            .unwrap();

        assert_eq!(
            mock.calls(),
            vec![Call::Subscribe {
                topic: "demo".to_string(),
                // Keyed by the observer id: the same id finds the same queue.
                id: "observer:obs-1".to_string(),
                subscriber: "obs-1".to_string(),
                queue_config: Some(SubscriberQueueConfig {
                    max_retries: Some(2),
                    backoff_delay_ms: Some(1),
                    ..Default::default()
                }),
            }]
        );
    }

    #[test]
    fn subscription_identity_keeps_groups_and_observers_apart() {
        let observer = registration(r"obs@v1\path", json!({"topic": "demo"}));
        assert_eq!(observer.subscription_key(), r"observer:obs@v1\path");
        let mut grouped = observer.clone();
        grouped.spec.group = Some(r"bill@ing\x".to_string());
        assert_eq!(grouped.subscription_key(), r"@bill\@ing\\x");
        // An observer whose id starts with `@` never meets a group's key.
        let lookalike = registration("@billing", json!({"topic": "demo"}));
        assert_ne!(
            lookalike.subscription_key(),
            registration("x", json!({"topic": "demo", "group": "billing"})).subscription_key()
        );
    }

    /// A consumer group is one subscription, whatever its members, and it
    /// keeps delivering when the member it delivered into leaves while others
    /// stay.
    #[tokio::test]
    async fn a_group_is_one_subscription_and_outlives_a_member() {
        let (subscribers, mock) = subscribers_with_mock();
        let group = json!({"topic": "demo", "group": "billing"});
        subscribers
            .register(registration("obs-a", group.clone()))
            .await
            .unwrap();
        subscribers
            .register(registration("obs-b", group))
            .await
            .unwrap();
        assert_eq!(
            mock.calls().len(),
            1,
            "the second member shares the subscription"
        );
        assert!(matches!(
            &mock.calls()[0],
            Call::Subscribe { id, subscriber, .. } if id == "@billing" && subscriber == "obs-a"
        ));

        subscribers.unregister("obs-a").await;
        let calls = mock.calls();
        assert!(matches!(&calls[1], Call::Unsubscribe { id, .. } if id == "@billing"));
        assert!(matches!(
            &calls[2],
            Call::Subscribe { id, subscriber, .. } if id == "@billing" && subscriber == "obs-b"
        ));

        subscribers.unregister("obs-b").await;
        assert!(matches!(
            mock.calls().last().unwrap(),
            Call::Unsubscribe { id, .. } if id == "@billing"
        ));
    }

    /// A member that leaves while another one is served changes nothing.
    #[tokio::test]
    async fn a_member_not_served_leaves_the_subscription_alone() {
        let (subscribers, mock) = subscribers_with_mock();
        let group = json!({"topic": "demo", "group": "billing"});
        subscribers
            .register(registration("obs-a", group.clone()))
            .await
            .unwrap();
        subscribers
            .register(registration("obs-b", group))
            .await
            .unwrap();
        subscribers.unregister("obs-b").await;
        assert_eq!(mock.calls().len(), 1, "still served by obs-a");
    }

    /// An observer that comes back under the same id (its worker restarted
    /// with a pinned id) attaches to the same durable queue.
    #[tokio::test]
    async fn the_same_observer_id_keeps_the_subscription_key() {
        let (subscribers, mock) = subscribers_with_mock();
        subscribers
            .register(registration("orders-worker", json!({"topic": "demo"})))
            .await
            .unwrap();
        subscribers.unregister("orders-worker").await;
        subscribers
            .register(registration("orders-worker", json!({"topic": "demo"})))
            .await
            .unwrap();

        let ids: Vec<_> = mock
            .calls()
            .iter()
            .map(|call| match call {
                Call::Subscribe { id, .. } => format!("subscribe:{id}"),
                Call::Unsubscribe { id, .. } => format!("unsubscribe:{id}"),
            })
            .collect();
        assert_eq!(
            ids,
            vec![
                "subscribe:observer:orders-worker",
                "unsubscribe:observer:orders-worker",
                "subscribe:observer:orders-worker"
            ],
            "both registrations must attach to the same durable queue"
        );
    }

    /// A hot-swap resubscribe collapses a group's members to ONE subscribe
    /// against the new adapter, mirroring `register`'s dedup — otherwise the
    /// swap would stack duplicate consumers that each deliver every message
    /// again.
    #[tokio::test]
    async fn resubscribe_all_subscribes_shared_keys_once() {
        let (subscribers, _old_mock) = subscribers_with_mock();
        let group = json!({"topic": "demo", "group": "billing"});
        subscribers
            .register(registration("obs-a", group.clone()))
            .await
            .unwrap();
        subscribers
            .register(registration("obs-b", group))
            .await
            .unwrap();

        let new_mock = Arc::new(MockAdapter::default());
        let new_adapter: Arc<dyn QueueAdapter> = new_mock.clone();
        subscribers.adapter.replace(new_adapter, "new-mock").await;
        subscribers.resubscribe_all().await;

        assert_eq!(
            new_mock.calls().len(),
            1,
            "shared-key registrations must resubscribe once"
        );
        let Call::Subscribe { id, .. } = &new_mock.calls()[0] else {
            panic!("expected a Subscribe call");
        };
        assert_eq!(id, "@billing");
    }

    #[tokio::test]
    async fn register_rejects_empty_topic() {
        let (subscribers, _mock) = subscribers_with_mock();
        let err = subscribers
            .register(registration("obs-1", json!({"topic": ""})))
            .await
            .unwrap_err();
        assert!(matches!(err, RegisterError::InvalidConfig(_)));
        assert!(err.to_string().contains("topic"));
    }

    #[tokio::test]
    async fn duplicate_observer_id_is_rejected() {
        let (subscribers, mock) = subscribers_with_mock();
        subscribers
            .register(registration("obs-1", json!({"topic": "demo"})))
            .await
            .unwrap();
        let err = subscribers
            .register(registration("obs-1", json!({"topic": "demo"})))
            .await
            .unwrap_err();
        assert!(matches!(err, RegisterError::AlreadyRegistered(_)));
        assert!(err.to_string().contains("already registered"));
        assert_eq!(mock.calls().len(), 1, "adapter.subscribe called only once");
    }

    #[tokio::test]
    async fn unregister_by_id_calls_adapter_unsubscribe() {
        let (subscribers, mock) = subscribers_with_mock();
        subscribers
            .register(registration("obs-1", json!({"topic": "demo"})))
            .await
            .unwrap();

        subscribers.unregister("obs-1").await;

        assert_eq!(
            mock.calls().last().unwrap(),
            &Call::Unsubscribe {
                topic: "demo".to_string(),
                id: "observer:obs-1".to_string(),
                retain: Duration::from_secs(30),
            }
        );
        assert!(subscribers.registrations().await.is_empty());
    }

    /// The retention an observer's leaving passes is the live configuration's
    /// `subscriber_retention_ms`, read when it leaves.
    #[tokio::test]
    async fn unregister_passes_the_configured_retention() {
        let (subscribers, mock) = subscribers_with_mock();
        let config = crate::configuration::new_cell(crate::config::QueueConfig {
            subscriber_retention_ms: 0,
            ..Default::default()
        });
        let subscribers = subscribers.with_config(config.clone());
        for id in ["obs-1", "obs-2"] {
            subscribers
                .register(registration(id, json!({"topic": "demo"})))
                .await
                .unwrap();
        }

        subscribers.unregister("obs-1").await;
        *config.write().await = Arc::new(crate::config::QueueConfig {
            subscriber_retention_ms: 1_500,
            ..Default::default()
        });
        subscribers.unregister("obs-2").await;

        let retained: Vec<_> = mock
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Unsubscribe { id, retain, .. } => Some((id, retain)),
                Call::Subscribe { .. } => None,
            })
            .collect();
        assert_eq!(
            retained,
            vec![
                ("observer:obs-1".to_string(), Duration::ZERO),
                ("observer:obs-2".to_string(), Duration::from_millis(1_500)),
            ]
        );
    }

    #[tokio::test]
    async fn resubscribe_all_attaches_every_registration_to_the_new_adapter() {
        let (subscribers, old_mock) = subscribers_with_mock();
        subscribers
            .register(registration(
                "obs-1",
                json!({"topic": "demo-1", "max_retries": 2}),
            ))
            .await
            .unwrap();
        subscribers
            .register(registration("obs-2", json!({"topic": "demo-2"})))
            .await
            .unwrap();
        assert_eq!(old_mock.calls().len(), 2, "initial registration subscribes");

        // Simulate a hot-swap: the adapter behind the registry's
        // `SwappableAdapter` is replaced with a fresh mock.
        let new_mock = Arc::new(MockAdapter::default());
        let new_adapter: Arc<dyn QueueAdapter> = new_mock.clone();
        subscribers.adapter.replace(new_adapter, "new-mock").await;

        subscribers.resubscribe_all().await;

        // The old adapter never sees another call post-swap.
        assert_eq!(old_mock.calls().len(), 2);
        // The new adapter is subscribed once per tracked registration.
        let new_calls = new_mock.calls();
        assert_eq!(new_calls.len(), 2);
        assert!(new_calls.contains(&Call::Subscribe {
            topic: "demo-1".to_string(),
            id: "observer:obs-1".to_string(),
            subscriber: "obs-1".to_string(),
            queue_config: Some(SubscriberQueueConfig {
                max_retries: Some(2),
                ..Default::default()
            }),
        }));
        assert!(new_calls.contains(&Call::Subscribe {
            topic: "demo-2".to_string(),
            id: "observer:obs-2".to_string(),
            subscriber: "obs-2".to_string(),
            queue_config: Some(SubscriberQueueConfig::default()),
        }));
        // The registrations map itself is untouched by the swap.
        assert_eq!(subscribers.registrations().await.len(), 2);
    }

    #[tokio::test]
    async fn unregister_unknown_id_is_a_noop() {
        let (subscribers, mock) = subscribers_with_mock();
        subscribers.unregister("missing").await;
        assert!(mock.calls().is_empty());
    }

    #[test]
    fn supports_topic_and_queue_config_aliases() {
        let spec: SubscriberSpec = serde_json::from_value(json!({
            "topic": "demo",
            "queue_config": {
                "maxRetries": 9,
                "backoffDelayMs": 25
            }
        }))
        .unwrap();
        assert_eq!(spec.queue, "demo");
        let merged = merged_queue_config(&spec);
        assert_eq!(merged.max_retries, Some(9));
        assert_eq!(merged.backoff_delay_ms, Some(25));
    }

    #[test]
    fn top_level_overrides_win_over_queue_config() {
        let spec = SubscriberSpec {
            queue: "demo".to_string(),
            max_retries: Some(2),
            backoff_ms: Some(1),
            queue_config: Some(SubscriberQueueConfig {
                max_retries: Some(9),
                backoff_delay_ms: Some(25),
                ..Default::default()
            }),
            group: None,
        };
        let merged = merged_queue_config(&spec);
        assert_eq!(merged.max_retries, Some(2));
        assert_eq!(merged.backoff_delay_ms, Some(1));
    }

    /// The default `deliver` hands the message to `call` under the
    /// subscriber id, which test invokers record.
    #[tokio::test]
    async fn a_subscriber_delivers_through_the_invoker() {
        #[derive(Default)]
        struct Recording(StdMutex<Vec<(String, Value)>>);

        #[async_trait]
        impl Invoker for Recording {
            async fn call(
                &self,
                function_id: &str,
                payload: Value,
            ) -> Result<Option<Value>, String> {
                self.0
                    .lock()
                    .unwrap()
                    .push((function_id.to_string(), payload));
                Ok(Some(json!("acked")))
            }
        }

        let invoker = Recording::default();
        let out = Subscriber::new("obs-1")
            .deliver(&invoker, json!({"n": 1}))
            .await
            .unwrap();
        assert_eq!(out, Some(json!("acked")));
        assert_eq!(
            invoker.0.lock().unwrap().as_slice(),
            &[("obs-1".to_string(), json!({"n": 1}))]
        );
    }
}
