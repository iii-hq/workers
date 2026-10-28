//! Owned event feeds: the `devin::agent-event` trigger type carries the
//! translated AgentEvent frames and `devin::raw-event` the raw Devin CLI
//! stdout lines (`{ type: "stdout", line }`). The worker registers both at startup and delivers each
//! frame to the bindings for its session; nothing goes through a stream.
//!
//! Contract (shared by every agent worker's feed):
//! - binding config `{ session_id, metadata? }`: `session_id` is required,
//!   non-empty and at most 512 chars; `metadata` is an optional object that
//!   wins over the binding's own metadata; any other key, or a malformed
//!   value, rejects the binding. At most [`MAX_BINDINGS`] bindings per trigger
//!   type; re-registering an existing binding id replaces it.
//! - delivery payload `{ session_id, event_id, seq, epoch, source, event }`:
//!   `event` is the frame unchanged, `seq` is contiguous per (feed, session,
//!   epoch) starting at 0 with a separate counter per feed, `epoch` is a
//!   per-process uuid, `event_id` = `<session_id>-<epoch>-<seq:08>` and
//!   `source` = [`SOURCE`].
//! - every binding for the event's session gets one fire-and-forget (Void)
//!   trigger call, sequentially, with the binding's function id, namespace and
//!   metadata; failures are logged and swallowed so a turn never fails because
//!   a consumer is gone. No matching binding means no call at all.
//! - ephemeral: frames are not stored or replayed. Consumers order by
//!   `(epoch, seq)` and dedup by `event_id`; history lives in the session
//!   record and the run result.
//!
//! The binding table and counters live behind a `std::sync::Mutex`; matching
//! bindings are cloned out before any trigger call is awaited.
//!
//! iii 0.25: each feed also has an owner-delivered observable function,
//! `devin::on-agent-event` and `devin::on-raw-event`. An observer passes the
//! binding config as args (`{ session_id, metadata? }`, same validation; the
//! `metadata` key is accepted and unused, an observer delivery carries no
//! trigger metadata), is filed in the same feed as the bindings, and receives
//! the same payload with the same `seq` through `obs.call`, after the
//! bindings. Frames are routed in-process; invoking either function directly
//! is refused with FORBIDDEN. Both trigger types stay registered for old
//! consumers on 0.24 and 0.25 engines.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerAction, TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
    RegisterTriggerType,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Producer namespace stamped on every delivery as `source`.
pub const SOURCE: &str = "devin";
/// Trigger type carrying the translated AgentEvent frames.
pub const AGENT_EVENT_TRIGGER_TYPE: &str = "devin::agent-event";
/// Observable function next to [`AGENT_EVENT_TRIGGER_TYPE`] (iii 0.25).
pub const ON_AGENT_EVENT_FN: &str = "devin::on-agent-event";
/// Trigger type carrying the raw Devin CLI stdout lines, verbatim.
pub const RAW_EVENT_TRIGGER_TYPE: &str = "devin::raw-event";
/// Observable function next to [`RAW_EVENT_TRIGGER_TYPE`] (iii 0.25).
pub const ON_RAW_EVENT_FN: &str = "devin::on-raw-event";
/// Bindings accepted per trigger type, and observers per observable function
/// (counted apart); a new one beyond it is rejected.
pub const MAX_BINDINGS: usize = 256;
/// Longest accepted `session_id`, in characters.
pub const MAX_SESSION_ID_CHARS: usize = 512;

const AGENT_EVENT_DESCRIPTION: &str = "Translated AgentEvent frames of one devin session. \
     Config: { session_id, metadata? }. Payload: { session_id, event_id, seq, epoch, source, \
     event }. Ephemeral (not stored, not replayed); order by (epoch, seq).";
const RAW_EVENT_DESCRIPTION: &str = "Raw Devin CLI stdout lines ({ type: stdout, line }, \
     verbatim) of one devin session. Config: { session_id, metadata? }. Payload: { session_id, \
     event_id, seq, epoch, source, event }. Ephemeral (not stored, not replayed); order by \
     (epoch, seq).";
const ON_AGENT_EVENT_DESCRIPTION: &str = "Translated AgentEvent frames of one devin session. \
     Observe it with args { session_id, metadata? } (the devin::agent-event config); each \
     observer of the session receives { session_id, event_id, seq, epoch, source, event }. \
     Ephemeral (not stored, not replayed); order by (epoch, seq). Only the devin worker \
     delivers it: invoking it directly is refused.";
const ON_RAW_EVENT_DESCRIPTION: &str = "Raw Devin CLI stdout lines ({ type: stdout, line }, \
     verbatim) of one devin session. Observe it with args { session_id, metadata? } (the \
     devin::raw-event config); each observer of the session receives { session_id, event_id, \
     seq, epoch, source, event }. Ephemeral (not stored, not replayed); order by (epoch, seq). \
     Only the devin worker delivers it: invoking it directly is refused.";

/// Codes an observer is refused with, and the direct-invocation refusal.
pub mod codes {
    /// Any direct invocation of an observable feed function.
    pub const FORBIDDEN: &str = "FORBIDDEN";
    /// Args the binding-config validation refuses (an empty or too long
    /// `session_id`, a null `metadata`).
    pub const INVALID_ARGS: &str = "INVALID_ARGS";
    /// The function already holds [`super::MAX_BINDINGS`] observers.
    pub const OBSERVER_LIMIT: &str = "OBSERVER_LIMIT";
    /// SDK codes of an `obs.call` that sent nothing: not delivery failures.
    pub(super) const FILTERS_NOT_MATCHED: &str = "FILTERS_NOT_MATCHED";
    pub(super) const OBSERVER_NOT_ACTIVE: &str = "OBSERVER_NOT_ACTIVE";
}

/// An observer's args: the trigger types' binding config. The engine checks
/// them against this schema; [`validate_feed_config`] then applies the
/// binding rules.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedArgs {
    /// The devin session to follow (non-empty, at most 512 characters).
    pub session_id: String,
    /// Accepted for parity with the binding config; unused for observers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

/// What a binding and an observer receive per frame.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct FeedEvent {
    pub session_id: String,
    /// `<session_id>-<epoch>-<seq:08>`: the dedup key.
    pub event_id: String,
    /// Contiguous per (feed, session, epoch), starting at 0.
    pub seq: u64,
    /// Per-process uuid.
    pub epoch: String,
    /// Always `devin`.
    pub source: String,
    /// The frame, unchanged.
    pub event: Value,
}

/// A validated binding config.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedConfig {
    pub session_id: String,
    pub metadata: Option<Value>,
}

/// Validate a binding config; the error message rejects the binding.
pub fn validate_feed_config(type_id: &str, raw: &Value) -> Result<FeedConfig, String> {
    let obj = raw.as_object().ok_or_else(|| {
        format!("{type_id}: config must be an object {{ session_id, metadata? }}")
    })?;
    if let Some(key) = obj
        .keys()
        .find(|key| key.as_str() != "session_id" && key.as_str() != "metadata")
    {
        return Err(format!(
            "{type_id}: unknown config key `{key}` (allowed: session_id, metadata)"
        ));
    }
    let session_id = match obj.get("session_id") {
        Some(Value::String(s)) if s.is_empty() => {
            return Err(format!("{type_id}: session_id must not be empty"))
        }
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(format!("{type_id}: session_id must be a string")),
        None => return Err(format!("{type_id}: session_id is required")),
    };
    if session_id.chars().count() > MAX_SESSION_ID_CHARS {
        return Err(format!(
            "{type_id}: session_id is longer than {MAX_SESSION_ID_CHARS} characters"
        ));
    }
    let metadata = match obj.get("metadata") {
        None => None,
        Some(value @ Value::Object(_)) => Some(value.clone()),
        Some(_) => return Err(format!("{type_id}: metadata must be an object")),
    };
    Ok(FeedConfig {
        session_id,
        metadata,
    })
}

#[derive(Debug, Clone)]
struct Binding {
    function_id: String,
    namespace: Option<String>,
    metadata: Option<Value>,
    config: FeedConfig,
}

/// One prepared trigger call: target, namespace, metadata and payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    pub function_id: String,
    pub namespace: Option<String>,
    pub metadata: Option<Value>,
    pub payload: Value,
}

impl Delivery {
    /// The fire-and-forget (Void) trigger request for this delivery.
    pub fn into_request(self) -> TriggerRequestWithMetadata {
        let mut request: TriggerRequestWithMetadata = TriggerRequest {
            function_id: self.function_id,
            payload: self.payload,
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        }
        .into();
        if let Some(namespace) = self.namespace {
            request = request.namespace(namespace);
        }
        if let Some(metadata) = self.metadata {
            request = request.metadata(metadata);
        }
        request
    }
}

/// One observer of a feed's observable function, as the feed delivers to it:
/// [`Observer`] delivers with `obs.call`.
#[async_trait]
pub trait FeedObserver: Send + Sync {
    fn id(&self) -> &str;
    /// Hand one delivery payload to the observer's target.
    async fn deliver(&self, payload: Value) -> Result<(), Error>;
}

#[async_trait]
impl FeedObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    async fn deliver(&self, payload: Value) -> Result<(), Error> {
        self.call::<Value>(payload).await.map(|_| ())
    }
}

/// An observer's filters rejected the payload, or it was removed meanwhile:
/// nothing was sent, which is not a delivery failure.
fn is_skipped_delivery(error: &Error) -> bool {
    matches!(
        error,
        Error::Remote { code, .. }
            if code == codes::FILTERS_NOT_MATCHED || code == codes::OBSERVER_NOT_ACTIVE
    )
}

#[derive(Clone)]
struct FeedObserverEntry {
    session_id: String,
    sink: Arc<dyn FeedObserver>,
}

#[derive(Default)]
struct FeedState {
    bindings: HashMap<String, Binding>,
    observers: HashMap<String, FeedObserverEntry>,
    seq_by_session: HashMap<String, u64>,
}

/// One frame's matching bindings and observers, and the payload they share.
struct Prepared {
    deliveries: Vec<Delivery>,
    observers: Vec<Arc<dyn FeedObserver>>,
    payload: Value,
}

/// One owned trigger type: its bindings and per-session sequence counters.
pub struct Feed {
    id: String,
    source: String,
    epoch: String,
    state: Mutex<FeedState>,
}

impl Feed {
    pub fn new(id: impl Into<String>, source: impl Into<String>, epoch: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            epoch: epoch.into(),
            state: Mutex::new(FeedState::default()),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    fn lock(&self) -> MutexGuard<'_, FeedState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Validate and store a binding (replacing one with the same id).
    pub fn bind(&self, binding: &TriggerConfig) -> Result<(), String> {
        let config = validate_feed_config(&self.id, &binding.config)?;
        let mut state = self.lock();
        if !state.bindings.contains_key(&binding.id) && state.bindings.len() >= MAX_BINDINGS {
            return Err(format!(
                "{}: binding limit ({MAX_BINDINGS}) reached",
                self.id
            ));
        }
        state.bindings.insert(
            binding.id.clone(),
            Binding {
                function_id: binding.function_id.clone(),
                namespace: binding.namespace.clone(),
                metadata: binding.metadata.clone(),
                config,
            },
        );
        Ok(())
    }

    /// Remove a binding; true when it existed.
    pub fn unbind(&self, binding_id: &str) -> bool {
        self.lock().bindings.remove(binding_id).is_some()
    }

    pub fn binding_count(&self) -> usize {
        self.lock().bindings.len()
    }

    /// File an observer of this feed's observable function under its args,
    /// validated as a binding config (replacing one with the same id).
    pub fn add_observer(
        &self,
        args: &Value,
        observer: Arc<dyn FeedObserver>,
    ) -> Result<(), ObserveRejection> {
        let config = validate_feed_config(&self.id, args)
            .map_err(|message| ObserveRejection::new(codes::INVALID_ARGS, message))?;
        let id = observer.id().to_string();
        let mut state = self.lock();
        if !state.observers.contains_key(&id) && state.observers.len() >= MAX_BINDINGS {
            return Err(ObserveRejection::new(
                codes::OBSERVER_LIMIT,
                format!("{}: observer limit ({MAX_BINDINGS}) reached", self.id),
            ));
        }
        state.observers.insert(
            id,
            FeedObserverEntry {
                session_id: config.session_id,
                sink: observer,
            },
        );
        Ok(())
    }

    /// Remove an observer; true when it was filed.
    pub fn remove_observer(&self, observer_id: &str) -> bool {
        self.lock().observers.remove(observer_id).is_some()
    }

    pub fn observer_count(&self) -> usize {
        self.lock().observers.len()
    }

    /// Assign the next sequence number for `session_id` on this feed and
    /// build one delivery per matching binding (empty when none match). The
    /// lock is released before this returns. Observers are delivered by
    /// [`Feed::emit_with`].
    pub fn prepare(&self, session_id: &str, event: Value) -> Vec<Delivery> {
        self.prepare_all(session_id, event)
            .map(|prepared| prepared.deliveries)
            .unwrap_or_default()
    }

    /// Assign the next sequence number for `session_id` and collect the
    /// matching bindings and observers; `None` when nothing matches. The lock
    /// is released before this returns.
    fn prepare_all(&self, session_id: &str, event: Value) -> Option<Prepared> {
        let (seq, targets, observers) = {
            let mut state = self.lock();
            let counter = state
                .seq_by_session
                .entry(session_id.to_string())
                .or_insert(0);
            let seq = *counter;
            *counter += 1;
            let targets: Vec<Binding> = state
                .bindings
                .values()
                .filter(|binding| binding.config.session_id == session_id)
                .cloned()
                .collect();
            let observers: Vec<Arc<dyn FeedObserver>> = state
                .observers
                .values()
                .filter(|entry| entry.session_id == session_id)
                .map(|entry| entry.sink.clone())
                .collect();
            (seq, targets, observers)
        };
        if targets.is_empty() && observers.is_empty() {
            return None;
        }
        let payload = json!({
            "session_id": session_id,
            "event_id": format!("{session_id}-{}-{seq:08}", self.epoch),
            "seq": seq,
            "epoch": self.epoch,
            "source": self.source,
            "event": event,
        });
        let deliveries = targets
            .into_iter()
            .map(|binding| Delivery {
                function_id: binding.function_id,
                namespace: binding.namespace,
                metadata: binding.config.metadata.or(binding.metadata),
                payload: payload.clone(),
            })
            .collect();
        Some(Prepared {
            deliveries,
            observers,
            payload,
        })
    }

    /// Deliver one frame through `send` to each matching binding, then with
    /// `obs.call` to each matching observer, sequentially; failures are logged
    /// and swallowed. Returns how many deliveries succeeded (bindings and
    /// observers together).
    pub async fn emit_with<F, Fut>(&self, session_id: &str, event: Value, mut send: F) -> usize
    where
        F: FnMut(Delivery) -> Fut,
        Fut: Future<Output = Result<Value, Error>>,
    {
        let Some(prepared) = self.prepare_all(session_id, event) else {
            return 0;
        };
        let mut delivered = 0;
        for delivery in prepared.deliveries {
            let function_id = delivery.function_id.clone();
            match send(delivery).await {
                Ok(_) => delivered += 1,
                Err(e) => tracing::warn!(
                    trigger_type = %self.id,
                    function_id = %function_id,
                    session_id,
                    error = %e,
                    "event feed delivery failed"
                ),
            }
        }
        for observer in prepared.observers {
            match observer.deliver(prepared.payload.clone()).await {
                Ok(()) => delivered += 1,
                Err(e) if is_skipped_delivery(&e) => tracing::debug!(
                    trigger_type = %self.id,
                    observer = %observer.id(),
                    session_id,
                    error = %e,
                    "event feed observer skipped the frame"
                ),
                Err(e) => tracing::warn!(
                    trigger_type = %self.id,
                    observer = %observer.id(),
                    session_id,
                    error = %e,
                    "event feed observer delivery failed"
                ),
            }
        }
        delivered
    }

    /// Deliver one frame to the matching bindings through the engine.
    pub async fn emit(&self, iii: &IIIClient, session_id: &str, event: Value) -> usize {
        self.emit_with(session_id, event, |delivery| {
            iii.trigger(delivery.into_request())
        })
        .await
    }
}

/// The engine-facing handler of one feed's trigger type.
pub struct FeedHandler {
    feed: Arc<Feed>,
}

impl FeedHandler {
    pub fn new(feed: Arc<Feed>) -> Self {
        Self { feed }
    }
}

#[async_trait]
impl TriggerHandler for FeedHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.feed.bind(&config).map_err(|message| {
            tracing::warn!(trigger_type = %self.feed.id, binding = %config.id, %message, "binding rejected");
            Error::Handler(message)
        })
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.feed.unbind(&config.id);
        Ok(())
    }
}

/// The observe hooks of one feed's observable function: each observer is
/// filed in the same feed as the trigger type's bindings, under its
/// `session_id`, and taken out when the engine removes it.
pub struct FeedObserveHandler {
    feed: Arc<Feed>,
}

impl FeedObserveHandler {
    pub fn new(feed: Arc<Feed>) -> Self {
        Self { feed }
    }
}

#[async_trait]
impl ObserveHandler for FeedObserveHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        // Filed before the ack: the first frame after the engine marks the
        // observer active reaches it.
        self.feed
            .add_observer(obs.args_value(), Arc::new(obs.clone()))
            .inspect_err(|rejection| {
                tracing::warn!(trigger_type = %self.feed.id, observer = %obs.id(), message = %rejection.message, "observer rejected");
            })?;
        tracing::info!(trigger_type = %self.feed.id, observer = %obs.id(), target = %obs.target(), "feed observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.feed.remove_observer(obs.id());
        tracing::info!(trigger_type = %self.feed.id, observer = %obs.id(), "feed observer removed");
    }
}

/// The refusal every direct invocation of an observable feed function gets:
/// frames only come from the devin turn that produced them, routed in-process.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: codes::FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args: {{ session_id }} }}, target)); \
             only the devin worker delivers its events"
        ),
        stacktrace: None,
    }
}

/// Handler of `devin::on-agent-event`: any payload is refused.
fn refuse_agent_event(_payload: Value) -> Result<(), Error> {
    Err(refuse_invocation(ON_AGENT_EVENT_FN))
}

/// Handler of `devin::on-raw-event`: any payload is refused.
fn refuse_raw_event(_payload: Value) -> Result<(), Error> {
    Err(refuse_invocation(ON_RAW_EVENT_FN))
}

/// The worker's two feeds; both share the process epoch, each keeps its own
/// counters.
pub struct Feeds {
    pub agent: Arc<Feed>,
    pub raw: Arc<Feed>,
}

impl Feeds {
    pub fn new(epoch: &str) -> Self {
        Self {
            agent: Arc::new(Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, epoch)),
            raw: Arc::new(Feed::new(RAW_EVENT_TRIGGER_TYPE, SOURCE, epoch)),
        }
    }
}

/// The process-wide feeds (epoch = a uuid minted once per process).
pub fn feeds() -> &'static Feeds {
    static FEEDS: OnceLock<Feeds> = OnceLock::new();
    FEEDS.get_or_init(|| Feeds::new(&uuid::Uuid::new_v4().to_string()))
}

/// Register both trigger types and both observable functions with the engine.
/// Call once at startup. The trigger types are deprecated in iii 0.25 and stay
/// for old consumers on 0.24 and 0.25 engines; on an engine without observe
/// the functions register as plain functions and their hooks never run.
pub fn register(iii: &IIIClient) {
    let feeds = feeds();
    #[allow(deprecated)]
    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        AGENT_EVENT_TRIGGER_TYPE,
        AGENT_EVENT_DESCRIPTION,
        FeedHandler::new(feeds.agent.clone()),
    ));
    #[allow(deprecated)]
    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        RAW_EVENT_TRIGGER_TYPE,
        RAW_EVENT_DESCRIPTION,
        FeedHandler::new(feeds.raw.clone()),
    ));
    let _ = iii.register_function(
        ON_AGENT_EVENT_FN,
        observable(
            RegisterFunction::new(refuse_agent_event),
            ON_AGENT_EVENT_DESCRIPTION,
            feeds.agent.clone(),
        ),
    );
    let _ = iii.register_function(
        ON_RAW_EVENT_FN,
        observable(
            RegisterFunction::new(refuse_raw_event),
            ON_RAW_EVENT_DESCRIPTION,
            feeds.raw.clone(),
        ),
    );
}

/// An owner-delivered observable function over `feed`: args_format is the
/// binding config, the request format the delivery payload.
fn observable(function: RegisterFunction, description: &str, feed: Arc<Feed>) -> RegisterFunction {
    function
        .description(description)
        .request_format(feed_event_schema())
        .observe(
            Observe::owner()
                .args_format::<FeedArgs>()
                .handler(FeedObserveHandler::new(feed)),
        )
}

fn feed_event_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(FeedEvent)).unwrap_or(Value::Null)
}

/// Deliver a translated AgentEvent frame on `devin::agent-event`.
pub async fn emit_agent_event(iii: &IIIClient, session_id: &str, event: Value) -> usize {
    feeds().agent.emit(iii, session_id, event).await
}

/// Deliver a raw CLI frame on `devin::raw-event`.
pub async fn emit_raw_event(iii: &IIIClient, session_id: &str, event: Value) -> usize {
    feeds().raw.emit(iii, session_id, event).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPOCH: &str = "epoch-test";

    fn binding(id: &str, function_id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: function_id.to_string(),
            config,
            metadata: None,
            namespace: None,
        }
    }

    /// Emit through a capturing sender; returns (delivered count, captured).
    async fn capture(feed: &Feed, session_id: &str, event: Value) -> (usize, Vec<Delivery>) {
        let sent = Mutex::new(Vec::new());
        let delivered = feed
            .emit_with(session_id, event, |delivery| {
                sent.lock().unwrap().push(delivery);
                std::future::ready(Ok::<Value, Error>(Value::Null))
            })
            .await;
        (delivered, sent.into_inner().unwrap())
    }

    #[tokio::test]
    async fn delivers_to_bound_consumer_with_namespace_metadata_and_void_action() {
        let feeds = Feeds::new(EPOCH);
        let handler = FeedHandler::new(feeds.agent.clone());
        let mut with_config_meta = binding(
            "b1",
            "acp::__on_event::c1",
            json!({ "session_id": "s1", "metadata": { "from": "config" } }),
        );
        with_config_meta.namespace = Some("tenant-a".into());
        with_config_meta.metadata = Some(json!({ "from": "binding" }));
        handler.register_trigger(with_config_meta).await.unwrap();
        let mut with_binding_meta = binding("b2", "other::fn", json!({ "session_id": "s1" }));
        with_binding_meta.metadata = Some(json!({ "from": "binding" }));
        handler.register_trigger(with_binding_meta).await.unwrap();

        let event = json!({ "type": "message_complete", "message": { "role": "assistant" } });
        let (delivered, mut sent) = capture(&feeds.agent, "s1", event.clone()).await;
        assert_eq!(delivered, 2);
        sent.sort_by(|a, b| a.function_id.cmp(&b.function_id));

        let first = &sent[0];
        assert_eq!(first.function_id, "acp::__on_event::c1");
        assert_eq!(first.namespace.as_deref(), Some("tenant-a"));
        assert_eq!(first.metadata, Some(json!({ "from": "config" })));
        assert_eq!(
            first.payload,
            json!({
                "session_id": "s1",
                "event_id": format!("s1-{EPOCH}-00000000"),
                "seq": 0,
                "epoch": EPOCH,
                "source": SOURCE,
                "event": event,
            })
        );
        assert_eq!(sent[1].function_id, "other::fn");
        assert_eq!(sent[1].namespace, None);
        assert_eq!(sent[1].metadata, Some(json!({ "from": "binding" })));

        // The engine request is fire-and-forget and carries namespace + metadata.
        let request = format!("{:?}", first.clone().into_request());
        assert!(request.contains("action: Some(Void)"), "{request}");
        assert!(
            request.contains("namespace: Some(\"tenant-a\")"),
            "{request}"
        );
        assert!(
            request.contains("\"from\": String(\"config\")"),
            "{request}"
        );
        assert!(
            request.contains("function_id: \"acp::__on_event::c1\""),
            "{request}"
        );
    }

    #[tokio::test]
    async fn metadata_is_omitted_when_neither_config_nor_binding_has_it() {
        let feed = Feed::new(RAW_EVENT_TRIGGER_TYPE, SOURCE, EPOCH);
        feed.bind(&binding("b", "f", json!({ "session_id": "s" })))
            .unwrap();
        let (_, sent) = capture(&feed, "s", json!({ "type": "turn.started" })).await;
        assert_eq!(sent[0].metadata, None);
        let request = format!("{:?}", sent[0].clone().into_request());
        assert!(request.contains("metadata: None"), "{request}");
    }

    #[tokio::test]
    async fn binding_for_one_session_never_sees_another() {
        let feed = Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, EPOCH);
        feed.bind(&binding("a", "fn::a", json!({ "session_id": "A" })))
            .unwrap();
        feed.bind(&binding("b", "fn::b", json!({ "session_id": "B" })))
            .unwrap();
        let (delivered, sent) = capture(&feed, "B", json!({ "type": "agent_end" })).await;
        assert_eq!(delivered, 1);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].function_id, "fn::b");
        assert_eq!(sent[0].payload["session_id"], "B");
        let (delivered, sent) = capture(&feed, "C", json!({ "type": "agent_end" })).await;
        assert_eq!(delivered, 0);
        assert!(sent.is_empty(), "no binding, no trigger call");
    }

    #[tokio::test]
    async fn no_delivery_after_unbind() {
        let feed = Arc::new(Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, EPOCH));
        let handler = FeedHandler::new(feed.clone());
        let config = binding("b", "fn::b", json!({ "session_id": "s" }));
        handler.register_trigger(config.clone()).await.unwrap();
        assert_eq!(capture(&feed, "s", json!({ "type": "x" })).await.0, 1);
        handler.unregister_trigger(config).await.unwrap();
        assert_eq!(feed.binding_count(), 0);
        let (delivered, sent) = capture(&feed, "s", json!({ "type": "x" })).await;
        assert_eq!(delivered, 0);
        assert!(sent.is_empty());
    }

    #[tokio::test]
    async fn invalid_configs_are_rejected() {
        let feed = Arc::new(Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, EPOCH));
        let handler = FeedHandler::new(feed.clone());
        let too_long = "x".repeat(MAX_SESSION_ID_CHARS + 1);
        for config in [
            json!({}),
            json!(null),
            json!("s1"),
            json!({ "session_id": "" }),
            json!({ "session_id": 7 }),
            json!({ "session_id": too_long }),
            json!({ "session_id": "s1", "group_id": "s1" }),
            json!({ "session_id": "s1", "metadata": "x" }),
            json!({ "session_id": "s1", "metadata": [1] }),
            json!({ "session_id": "s1", "metadata": null }),
        ] {
            let err = handler
                .register_trigger(binding("b", "f", config.clone()))
                .await
                .expect_err(&format!("{config} must be rejected"));
            assert!(matches!(err, Error::Handler(_)), "{err:?}");
        }
        assert_eq!(feed.binding_count(), 0);
        let max = "y".repeat(MAX_SESSION_ID_CHARS);
        assert!(validate_feed_config("t", &json!({ "session_id": max })).is_ok());
    }

    #[tokio::test]
    async fn binding_cap_rejects_new_ids_but_allows_replacing() {
        let feed = Arc::new(Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, EPOCH));
        let handler = FeedHandler::new(feed.clone());
        for i in 0..MAX_BINDINGS {
            handler
                .register_trigger(binding(&format!("b{i}"), "f", json!({ "session_id": "s" })))
                .await
                .unwrap();
        }
        assert_eq!(feed.binding_count(), MAX_BINDINGS);
        let err = handler
            .register_trigger(binding("overflow", "f", json!({ "session_id": "s" })))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("binding limit (256)"), "{err}");
        // Re-registering an existing id replaces it, even at the cap.
        handler
            .register_trigger(binding("b0", "replaced", json!({ "session_id": "other" })))
            .await
            .unwrap();
        assert_eq!(feed.binding_count(), MAX_BINDINGS);
        let (_, sent) = capture(&feed, "other", json!({ "type": "x" })).await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].function_id, "replaced");
    }

    #[tokio::test]
    async fn seq_is_contiguous_per_feed_and_session() {
        let feeds = Feeds::new(EPOCH);
        for feed in [&feeds.agent, &feeds.raw] {
            for session in ["s1", "s2"] {
                feed.bind(&binding(
                    &format!("{}-{session}", feed.id()),
                    "f",
                    json!({ "session_id": session }),
                ))
                .unwrap();
            }
        }
        let mut agent_s1 = Vec::new();
        let mut raw_s1 = Vec::new();
        for _ in 0..3 {
            raw_s1.extend(capture(&feeds.raw, "s1", json!({ "type": "raw" })).await.1);
            agent_s1.extend(capture(&feeds.agent, "s1", json!({ "type": "a" })).await.1);
        }
        raw_s1.extend(capture(&feeds.raw, "s1", json!({ "type": "raw" })).await.1);
        let agent_s2 = capture(&feeds.agent, "s2", json!({ "type": "a" })).await.1;

        let seqs = |sent: &[Delivery]| -> Vec<u64> {
            sent.iter()
                .map(|d| d.payload["seq"].as_u64().unwrap())
                .collect()
        };
        assert_eq!(seqs(&agent_s1), vec![0, 1, 2]);
        assert_eq!(seqs(&raw_s1), vec![0, 1, 2, 3]);
        assert_eq!(seqs(&agent_s2), vec![0]);
        assert_eq!(
            agent_s1[2].payload["event_id"],
            format!("s1-{EPOCH}-00000002")
        );
        assert_eq!(
            raw_s1[3].payload["event_id"],
            format!("s1-{EPOCH}-00000003")
        );
        assert_eq!(agent_s1[0].payload["epoch"], raw_s1[0].payload["epoch"]);
    }

    #[tokio::test]
    async fn seq_advances_without_bindings() {
        let feed = Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, EPOCH);
        assert!(feed.prepare("s", json!({ "type": "a" })).is_empty());
        feed.bind(&binding("b", "f", json!({ "session_id": "s" })))
            .unwrap();
        let sent = feed.prepare("s", json!({ "type": "a" }));
        assert_eq!(sent[0].payload["seq"], 1);
    }

    #[tokio::test]
    async fn failing_delivery_is_swallowed() {
        let feed = Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, EPOCH);
        feed.bind(&binding("bad", "fn::gone", json!({ "session_id": "s" })))
            .unwrap();
        feed.bind(&binding("good", "fn::live", json!({ "session_id": "s" })))
            .unwrap();
        let attempted = Mutex::new(Vec::new());
        let delivered = feed
            .emit_with("s", json!({ "type": "agent_end" }), |delivery| {
                attempted.lock().unwrap().push(delivery.function_id.clone());
                std::future::ready(if delivery.function_id == "fn::gone" {
                    Err(Error::NotConnected)
                } else {
                    Ok(Value::Null)
                })
            })
            .await;
        assert_eq!(delivered, 1);
        assert_eq!(attempted.into_inner().unwrap().len(), 2);
        // The next frame still flows with the next seq.
        let (delivered, sent) = capture(&feed, "s", json!({ "type": "x" })).await;
        assert_eq!(delivered, 2);
        assert_eq!(sent[0].payload["seq"], 1);
    }

    #[test]
    fn process_feeds_use_the_owned_trigger_type_ids() {
        let feeds = feeds();
        assert_eq!(feeds.agent.id(), "devin::agent-event");
        assert_eq!(feeds.raw.id(), "devin::raw-event");
        assert_eq!(feeds.agent.epoch(), feeds.raw.epoch());
        assert!(uuid::Uuid::parse_str(feeds.agent.epoch()).is_ok());
    }

    /// An observer of a feed's observable function that records its frames.
    struct RecordingObserver {
        id: String,
        received: Mutex<Vec<Value>>,
        fail_with: Option<&'static str>,
    }

    impl RecordingObserver {
        fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                received: Mutex::new(Vec::new()),
                fail_with: None,
            })
        }

        fn failing(id: &str, code: &'static str) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                received: Mutex::new(Vec::new()),
                fail_with: Some(code),
            })
        }

        fn received(&self) -> Vec<Value> {
            self.received.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl FeedObserver for RecordingObserver {
        fn id(&self) -> &str {
            &self.id
        }

        async fn deliver(&self, payload: Value) -> Result<(), Error> {
            if let Some(code) = self.fail_with {
                return Err(Error::Remote {
                    code: code.to_string(),
                    message: "not sent".to_string(),
                    stacktrace: None,
                });
            }
            self.received.lock().unwrap().push(payload);
            Ok(())
        }
    }

    #[test]
    fn observable_functions_refuse_direct_invocation() {
        for (function_id, refused) in [
            (
                ON_AGENT_EVENT_FN,
                refuse_agent_event(json!({ "session_id": "s1" })),
            ),
            (ON_RAW_EVENT_FN, refuse_raw_event(json!({}))),
            (ON_AGENT_EVENT_FN, refuse_agent_event(Value::Null)),
            (ON_RAW_EVENT_FN, refuse_raw_event(json!("forged"))),
        ] {
            let error = refused.unwrap_err();
            assert!(
                matches!(&error, Error::Remote { code, message, .. }
                    if code == codes::FORBIDDEN && message.contains(function_id)),
                "{error:?}"
            );
        }
        assert_eq!(ON_AGENT_EVENT_FN, "devin::on-agent-event");
        assert_eq!(ON_RAW_EVENT_FN, "devin::on-raw-event");
    }

    #[tokio::test]
    async fn an_observer_receives_the_frame_a_binding_receives() {
        let feeds = Feeds::new(EPOCH);
        let handler = FeedHandler::new(feeds.agent.clone());
        handler
            .register_trigger(binding("b1", "acp::fn", json!({ "session_id": "s1" })))
            .await
            .unwrap();
        let observer = RecordingObserver::new("o1");
        feeds
            .agent
            .add_observer(&json!({ "session_id": "s1" }), observer.clone())
            .unwrap();
        let elsewhere = RecordingObserver::new("o2");
        feeds
            .agent
            .add_observer(&json!({ "session_id": "s2" }), elsewhere.clone())
            .unwrap();

        let event = json!({ "type": "message_complete" });
        let (delivered, sent) = capture(&feeds.agent, "s1", event.clone()).await;
        assert_eq!(delivered, 2, "one binding and one observer");
        assert_eq!(sent.len(), 1);
        let received = observer.received();
        assert_eq!(received, vec![sent[0].payload.clone()]);
        let typed: FeedEvent = serde_json::from_value(received[0].clone()).unwrap();
        assert_eq!(typed.seq, 0);
        assert_eq!(typed.event, event);
        assert_eq!(typed.source, SOURCE);
        assert!(
            elsewhere.received().is_empty(),
            "another session's observer"
        );

        // The raw feed is its own: the agent observer sees none of it.
        capture(&feeds.raw, "s1", json!({ "raw": true })).await;
        assert_eq!(observer.received().len(), 1);
    }

    #[tokio::test]
    async fn an_observer_alone_gets_frames_and_shares_the_seq() {
        let feeds = Feeds::new(EPOCH);
        let observer = RecordingObserver::new("o1");
        feeds
            .raw
            .add_observer(&json!({ "session_id": "s1" }), observer.clone())
            .unwrap();
        for n in 0..3 {
            let (delivered, sent) = capture(&feeds.raw, "s1", json!({ "n": n })).await;
            assert_eq!(delivered, 1);
            assert!(sent.is_empty(), "no binding");
        }
        let seqs: Vec<u64> = observer
            .received()
            .iter()
            .map(|payload| payload["seq"].as_u64().unwrap())
            .collect();
        assert_eq!(seqs, [0, 1, 2]);

        assert!(feeds.raw.remove_observer("o1"));
        assert!(!feeds.raw.remove_observer("o1"));
        let (delivered, _) = capture(&feeds.raw, "s1", json!({ "n": 3 })).await;
        assert_eq!(delivered, 0);
        assert_eq!(observer.received().len(), 3);
    }

    #[tokio::test]
    async fn observer_args_follow_the_binding_rules_and_cap() {
        let feed = Feeds::new(EPOCH).agent;
        for bad in [
            json!({}),
            json!({ "session_id": "" }),
            json!({ "session_id": "x".repeat(MAX_SESSION_ID_CHARS + 1) }),
        ] {
            let rejection = feed
                .add_observer(&bad, RecordingObserver::new("bad"))
                .unwrap_err();
            assert_eq!(rejection.code, codes::INVALID_ARGS, "{bad}");
        }
        for n in 0..MAX_BINDINGS {
            feed.add_observer(
                &json!({ "session_id": "s1" }),
                RecordingObserver::new(&format!("o{n}")),
            )
            .unwrap();
        }
        let rejection = feed
            .add_observer(
                &json!({ "session_id": "s1" }),
                RecordingObserver::new("one-too-many"),
            )
            .unwrap_err();
        assert_eq!(rejection.code, codes::OBSERVER_LIMIT);
        // Replacing an existing observer id is not a new one.
        feed.add_observer(&json!({ "session_id": "s2" }), RecordingObserver::new("o0"))
            .unwrap();
        assert_eq!(feed.observer_count(), MAX_BINDINGS);
        assert_eq!(feed.binding_count(), 0, "observers are counted apart");
    }

    #[tokio::test]
    async fn a_skipped_or_failing_observer_is_swallowed() {
        let feeds = Feeds::new(EPOCH);
        let ok = RecordingObserver::new("ok");
        for observer in [
            RecordingObserver::failing("skipped", codes::FILTERS_NOT_MATCHED),
            RecordingObserver::failing("broken", "TARGET_FAILED"),
            ok.clone(),
        ] {
            feeds
                .agent
                .add_observer(&json!({ "session_id": "s1" }), observer)
                .unwrap();
        }
        let (delivered, _) = capture(&feeds.agent, "s1", json!({})).await;
        assert_eq!(delivered, 1);
        assert_eq!(ok.received().len(), 1);
    }
}
