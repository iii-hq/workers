//! The six session event functions this worker hosts, the internal
//! `session::store::on-events` feed, and the emission path behind them.
//!
//! Reactivity model (session-manager.md § Reactivity model): every
//! mutation calls the event function for what changed
//! (`session::on-created`, `session::on-message-added`, ...). Each one is
//! private (only this worker calls it) and its handler returns the event it
//! is called with, so its observers receive the event. Consumers observe
//! them with `iii.observe`; the calls are fire-and-forget
//! (`CallAction::Void`) and unordered, and an event can be lost, so
//! consumers reconcile via `revision` and the parent chain, never via
//! arrival order, and recover with a read-back.
//!
//! The functions take no args. iii applies each observer's filters to the
//! call: `input.session_id`, `input.message.role` (message events) and
//! `input.session_metadata.<key>`. Every event carries `session_metadata`,
//! the session's `SessionMeta.metadata` as of the mutation (for
//! `session::on-deleted`, as of just before deletion), so a tenancy filter
//! needs no lookup. The emitter calls a function only while something
//! observes it ([`EventDeliverer::is_observed`]), so an unobserved event is
//! never serialized, and append, append-many and update-message build their
//! event only when the sink wants it ([`EventSink::wants`]).
//!
//! Cross-instance propagation (bridge topology): the **main** (fs-mode)
//! instance is the single fan-out point. Besides its own event functions,
//! it calls [`STORE_ON_EVENTS_FN`] with an [`EventEnvelope`] for every
//! event; every bridged instance observes that function on the main's
//! engine with a relay function ([`attach_bridge_relay`]). A bridged
//! instance never emits locally at mutation time: its [`RemotePublisher`]
//! sends the envelopes to the main (`session::store::publish-events`), the
//! main runs them through its own [`Emitter`] (its own event functions plus
//! the envelope feed, which reaches *every* bridge, originator included),
//! and each bridge's relay calls its local event functions through its
//! local `Emitter`. One canonical path, no dedup needed, and observer
//! filters apply at every edge because the envelope carries the session
//! metadata.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{
    CallAction, CallRequest, Deliver, IIIClient, ObserverHandle, ObserverState, RegisterFunction,
    Source,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::types::{AgentMessage, CustomPayload, JsonMap, SessionStatus};

pub const ON_CREATED_FN: &str = "session::on-created";
pub const ON_MESSAGE_ADDED_FN: &str = "session::on-message-added";
pub const ON_MESSAGE_UPDATED_FN: &str = "session::on-message-updated";
pub const ON_STATUS_CHANGED_FN: &str = "session::on-status-changed";
pub const ON_META_UPDATED_FN: &str = "session::on-meta-updated";
pub const ON_DELETED_FN: &str = "session::on-deleted";

/// Internal: the envelope feed bridged instances observe. Registered on
/// every instance and called only by the main (fs-mode) instance.
pub const STORE_ON_EVENTS_FN: &str = "session::store::on-events";

/// Which of the six event functions an event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    Created,
    MessageAdded,
    MessageUpdated,
    StatusChanged,
    MetaUpdated,
    Deleted,
}

impl EventKind {
    /// The id of the function that emits this kind of event.
    pub fn function_id(&self) -> &'static str {
        match self {
            EventKind::Created => ON_CREATED_FN,
            EventKind::MessageAdded => ON_MESSAGE_ADDED_FN,
            EventKind::MessageUpdated => ON_MESSAGE_UPDATED_FN,
            EventKind::StatusChanged => ON_STATUS_CHANGED_FN,
            EventKind::MetaUpdated => ON_META_UPDATED_FN,
            EventKind::Deleted => ON_DELETED_FN,
        }
    }

    pub fn all() -> [EventKind; 6] {
        [
            EventKind::Created,
            EventKind::MessageAdded,
            EventKind::MessageUpdated,
            EventKind::StatusChanged,
            EventKind::MetaUpdated,
            EventKind::Deleted,
        ]
    }

    pub fn from_function_id(id: &str) -> Option<EventKind> {
        EventKind::all().into_iter().find(|k| k.function_id() == id)
    }
}

// ---------------------------------------------------------------------------
// Event payloads (what observers receive)
// ---------------------------------------------------------------------------

/// `session::on-created`: a new session exists (create, ensure or fork).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionCreatedEvent {
    pub session_id: String,
    pub title: String,
    pub description: String,
    /// `"idle"` on create.
    pub status: SessionStatus,
    /// Source session id when created by fork.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
    pub created_at: i64,
}

/// `session::on-message-added`: an entry was appended. Carries `message`
/// for `kind: "message"` entries, `custom` for `kind: "custom"` entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MessageAddedEvent {
    pub session_id: String,
    pub entry_id: String,
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<AgentMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomPayload>,
    /// Writer-supplied correlation (e.g. `{ turn_id }`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<JsonMap>,
    pub timestamp: i64,
}

/// `session::on-message-updated`: a message's content changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MessageUpdatedEvent {
    pub session_id: String,
    pub entry_id: String,
    /// The full updated message (consumers apply snapshots
    /// last-write-wins by `revision`).
    pub message: AgentMessage,
    /// Monotonic per entry; consumers keep the highest.
    pub revision: u64,
    /// Writer-supplied correlation (e.g. `{ turn_id }`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<JsonMap>,
    pub timestamp: i64,
}

/// `session::on-status-changed`: a session's status changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatusChangedEvent {
    pub session_id: String,
    pub status: SessionStatus,
    pub previous_status: SessionStatus,
    /// Short lifecycle detail retained on `working`, `done`, and `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    pub timestamp: i64,
}

/// `session::on-meta-updated`: title/description/metadata changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MetaUpdatedEvent {
    pub session_id: String,
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonMap>,
    pub timestamp: i64,
}

/// `session::on-deleted`: a session and its entries were removed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionDeletedEvent {
    pub session_id: String,
    pub timestamp: i64,
}

/// What an event function is called with and returns, so what its
/// observers observe: the event's fields plus the session's metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WithSessionMetadata<E> {
    #[serde(flatten)]
    pub event: E,
    /// `SessionMeta.metadata` as of the mutation (for `session::on-deleted`,
    /// as of just before deletion); omitted when the session has none.
    /// Observers filter on `input.session_metadata.<key>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_metadata: Option<JsonMap>,
}

/// A typed event produced by a mutation.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    Created(SessionCreatedEvent),
    MessageAdded(MessageAddedEvent),
    MessageUpdated(MessageUpdatedEvent),
    StatusChanged(StatusChangedEvent),
    MetaUpdated(MetaUpdatedEvent),
    Deleted(SessionDeletedEvent),
}

impl SessionEvent {
    pub fn kind(&self) -> EventKind {
        match self {
            SessionEvent::Created(_) => EventKind::Created,
            SessionEvent::MessageAdded(_) => EventKind::MessageAdded,
            SessionEvent::MessageUpdated(_) => EventKind::MessageUpdated,
            SessionEvent::StatusChanged(_) => EventKind::StatusChanged,
            SessionEvent::MetaUpdated(_) => EventKind::MetaUpdated,
            SessionEvent::Deleted(_) => EventKind::Deleted,
        }
    }

    pub fn session_id(&self) -> &str {
        match self {
            SessionEvent::Created(e) => &e.session_id,
            SessionEvent::MessageAdded(e) => &e.session_id,
            SessionEvent::MessageUpdated(e) => &e.session_id,
            SessionEvent::StatusChanged(e) => &e.session_id,
            SessionEvent::MetaUpdated(e) => &e.session_id,
            SessionEvent::Deleted(e) => &e.session_id,
        }
    }

    /// The event alone, without the session metadata.
    pub fn payload(&self) -> Value {
        match self {
            SessionEvent::Created(e) => serde_json::to_value(e),
            SessionEvent::MessageAdded(e) => serde_json::to_value(e),
            SessionEvent::MessageUpdated(e) => serde_json::to_value(e),
            SessionEvent::StatusChanged(e) => serde_json::to_value(e),
            SessionEvent::MetaUpdated(e) => serde_json::to_value(e),
            SessionEvent::Deleted(e) => serde_json::to_value(e),
        }
        .expect("session event payloads always serialize")
    }
}

/// An event paired with the session metadata snapshot it carries.
#[derive(Debug, Clone, PartialEq)]
pub struct EmittableEvent {
    pub event: SessionEvent,
    pub session_metadata: Option<JsonMap>,
}

impl EmittableEvent {
    /// What the event's function is called with: the event plus
    /// `session_metadata` when the session has metadata
    /// ([`WithSessionMetadata`]).
    pub fn payload(&self) -> Value {
        let mut payload = self.event.payload();
        if let (Some(metadata), Value::Object(fields)) = (&self.session_metadata, &mut payload) {
            fields.insert(
                "session_metadata".to_string(),
                Value::Object(metadata.clone()),
            );
        }
        payload
    }
}

/// Wire form of an [`EmittableEvent`] for cross-instance propagation: what
/// `session::store::on-events` observers receive, and what
/// `session::store::publish-events` takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventEnvelope {
    /// The event function the event belongs to (`session::on-created`, ...).
    pub function_id: String,
    /// The event, without the session metadata.
    pub payload: Value,
    /// `SessionMeta.metadata` as of the mutation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_metadata: Option<JsonMap>,
}

impl EventEnvelope {
    pub fn from_emittable(ev: &EmittableEvent) -> Self {
        Self {
            function_id: ev.event.kind().function_id().to_string(),
            payload: ev.event.payload(),
            session_metadata: ev.session_metadata.clone(),
        }
    }

    /// Lossless inverse of [`Self::from_emittable`]. Errors on unknown
    /// function ids or payloads that do not match the function's event.
    pub fn to_emittable(&self) -> Result<EmittableEvent, String> {
        let kind = EventKind::from_function_id(&self.function_id)
            .ok_or_else(|| format!("unknown event function {}", self.function_id))?;
        let payload = self.payload.clone();
        let parse_err =
            |e: serde_json::Error| format!("malformed {} envelope payload: {e}", self.function_id);
        let event = match kind {
            EventKind::Created => {
                SessionEvent::Created(serde_json::from_value(payload).map_err(parse_err)?)
            }
            EventKind::MessageAdded => {
                SessionEvent::MessageAdded(serde_json::from_value(payload).map_err(parse_err)?)
            }
            EventKind::MessageUpdated => {
                SessionEvent::MessageUpdated(serde_json::from_value(payload).map_err(parse_err)?)
            }
            EventKind::StatusChanged => {
                SessionEvent::StatusChanged(serde_json::from_value(payload).map_err(parse_err)?)
            }
            EventKind::MetaUpdated => {
                SessionEvent::MetaUpdated(serde_json::from_value(payload).map_err(parse_err)?)
            }
            EventKind::Deleted => {
                SessionEvent::Deleted(serde_json::from_value(payload).map_err(parse_err)?)
            }
        };
        Ok(EmittableEvent {
            event,
            session_metadata: self.session_metadata.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register one event function: private (a call from another worker would
/// be a forged event), no args, and a handler that returns the event it is
/// called with.
fn register_event_function<E>(iii: &IIIClient, id: &'static str, description: &'static str)
where
    E: Serialize + DeserializeOwned + JsonSchema + Send + 'static,
{
    iii.register_function(
        id,
        RegisterFunction::new(|event: WithSessionMetadata<E>| Ok::<_, Error>(event))
            .description(description)
            // Called on every mutation (every streaming delta for
            // message-updated): hidden by default in trace UIs
            // (workers/docs/sops/trace-hidden-functions.md).
            .metadata(json!({ "trace_hidden": true }))
            .private(),
    );
}

/// Register the six event functions and the internal
/// [`STORE_ON_EVENTS_FN`] feed. Both are registered whatever the adapter, so
/// a bridge <-> fs hot-reload needs no registration; only an fs-mode
/// instance calls the feed.
pub fn register_event_functions(iii: &IIIClient) {
    register_event_function::<SessionCreatedEvent>(
        iii,
        ON_CREATED_FN,
        "A session was created (session::create, session::ensure or session::fork). Observe it \
         with filters on input.session_id and input.session_metadata.<key>; the event carries the \
         session's metadata as session_metadata.",
    );
    register_event_function::<MessageAddedEvent>(
        iii,
        ON_MESSAGE_ADDED_FN,
        "An entry was appended to a session. Observe it with filters on input.session_id, \
         input.message.role and input.session_metadata.<key>; custom entries carry `custom` and \
         no message.",
    );
    register_event_function::<MessageUpdatedEvent>(
        iii,
        ON_MESSAGE_UPDATED_FN,
        "A message entry's content changed (e.g. streaming deltas); keep the highest revision per \
         entry. Observe it with filters on input.session_id, input.message.role and \
         input.session_metadata.<key>.",
    );
    register_event_function::<StatusChangedEvent>(
        iii,
        ON_STATUS_CHANGED_FN,
        "A session's status changed (idle/working/done/error). Observe it with filters on \
         input.session_id and input.session_metadata.<key>.",
    );
    register_event_function::<MetaUpdatedEvent>(
        iii,
        ON_META_UPDATED_FN,
        "A session's title/description/metadata changed. Observe it with filters on \
         input.session_id and input.session_metadata.<key>.",
    );
    register_event_function::<SessionDeletedEvent>(
        iii,
        ON_DELETED_FN,
        "A session and its entries were removed; session_metadata is the metadata it had before \
         deletion. Observe it with filters on input.session_id and \
         input.session_metadata.<key>.",
    );
    iii.register_function(
        STORE_ON_EVENTS_FN,
        RegisterFunction::new(|envelope: EventEnvelope| Ok::<_, Error>(envelope))
            .description(
                "Internal: the event-envelope feed bridged session-manager instances observe. \
                 Called by an fs-mode instance once per event. Observe the six session::on-* \
                 functions instead.",
            )
            .metadata(json!({ "internal": true, "trace_hidden": true }))
            .private(),
    );
    tracing::info!(
        functions = ?EventKind::all().map(|k| k.function_id()),
        feed = STORE_ON_EVENTS_FN,
        "registered session event functions"
    );
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// How the emitter reaches an event function. Production calls over the
/// bus; tests record.
#[async_trait]
pub trait EventDeliverer: Send + Sync {
    /// Whether anything observes `function_id`. The emitter builds and
    /// sends no payload for an unobserved function.
    fn is_observed(&self, function_id: &str) -> bool;
    /// Call `function_id` with `payload`.
    async fn call(&self, function_id: &str, payload: Value);
}

/// Fire-and-forget calls (`CallAction::Void`) so the mutation that produced
/// the event is never blocked on observer latency. Failures are logged and
/// swallowed: a misbehaving observer must not break the write path.
pub struct IiiDeliverer {
    iii: Arc<IIIClient>,
}

impl IiiDeliverer {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl EventDeliverer for IiiDeliverer {
    fn is_observed(&self, function_id: &str) -> bool {
        self.iii.is_observed(function_id)
    }

    async fn call(&self, function_id: &str, payload: Value) {
        // Every session mutation calls an event function, and its observers
        // (the console's live relays among them) run on every one: pure
        // bookkeeping in a trace, so the call rides an `iii.tag.hidden`
        // baggage scope. The spans it produces stack into the trace UI's
        // span-filter INTERNAL section, hidden by default
        // (workers/docs/sops/trace-hidden-functions.md).
        let res = iii_helpers::observability::run_with_baggage(
            &[("iii.tag.hidden", "session events")],
            self.iii.call(CallRequest {
                function_id: function_id.to_string(),
                payload,
                action: Some(CallAction::Void),
                timeout_ms: None,
            }),
        )
        .await;
        if let Err(e) = res {
            tracing::warn!(function_id, error = %e, "session event call failed");
        }
    }
}

/// Where a mutation's events go. fs mode publishes through the local
/// [`Emitter`]; bridge mode publishes to the main instance via
/// [`RemotePublisher`].
#[async_trait]
pub trait EventSink: Send + Sync {
    /// Whether an event of `kind` reaches anything. A mutation on the hot
    /// paths (append, append-many, update-message) builds its event only
    /// when this holds, so an unobserved streaming delta copies neither the
    /// message nor the session metadata.
    fn wants(&self, kind: EventKind) -> bool;
    async fn publish_all(&self, events: &[EmittableEvent]);
}

/// Calls each event's function, and on the main (fs-mode) instance the
/// [`STORE_ON_EVENTS_FN`] feed with its envelope. Each call happens only
/// while the function is observed.
pub struct Emitter {
    deliverer: Arc<dyn EventDeliverer>,
    /// Whether this emitter also calls [`STORE_ON_EVENTS_FN`] (main/fs
    /// mode). A bridged instance's emitter never does: its events reach the
    /// other instances through the main.
    feeds_bridges: bool,
}

impl Emitter {
    /// Emitter without the bridge feed (bridge mode and most tests).
    pub fn new(deliverer: Arc<dyn EventDeliverer>) -> Self {
        Self {
            deliverer,
            feeds_bridges: false,
        }
    }

    /// Emitter that also feeds [`STORE_ON_EVENTS_FN`] (main/fs mode).
    pub fn feeding_bridges(deliverer: Arc<dyn EventDeliverer>) -> Self {
        Self {
            deliverer,
            feeds_bridges: true,
        }
    }

    pub fn feeds_bridges(&self) -> bool {
        self.feeds_bridges
    }

    /// Whether [`Self::emit_all`] would call anything for an event of
    /// `kind`: its function is observed, or (main mode) a bridged instance
    /// observes the [`STORE_ON_EVENTS_FN`] feed.
    pub fn wants(&self, kind: EventKind) -> bool {
        self.deliverer.is_observed(kind.function_id())
            || (self.feeds_bridges && self.deliverer.is_observed(STORE_ON_EVENTS_FN))
    }

    pub async fn emit_all(&self, events: &[EmittableEvent]) {
        for ev in events {
            self.emit(ev).await;
            self.feed_bridges(ev).await;
        }
    }

    /// Ingest path for externally-produced envelopes (the main's
    /// `session::store::publish-events` and the bridges' relays).
    /// Returns the number of well-formed envelopes processed; malformed
    /// ones are skipped with a warning.
    pub async fn emit_envelopes(&self, envelopes: &[EventEnvelope]) -> usize {
        let mut accepted = 0;
        for envelope in envelopes {
            match envelope.to_emittable() {
                Ok(ev) => {
                    self.emit(&ev).await;
                    self.feed_bridges(&ev).await;
                    accepted += 1;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "dropping malformed event envelope");
                }
            }
        }
        accepted
    }

    /// Call the event's function, when anything observes it.
    async fn emit(&self, ev: &EmittableEvent) {
        let function_id = ev.event.kind().function_id();
        if !self.deliverer.is_observed(function_id) {
            return;
        }
        self.deliverer.call(function_id, ev.payload()).await;
    }

    /// Call the bridge feed with the event's envelope (main mode), when any
    /// bridged instance observes it.
    async fn feed_bridges(&self, ev: &EmittableEvent) {
        if !self.feeds_bridges || !self.deliverer.is_observed(STORE_ON_EVENTS_FN) {
            return;
        }
        let envelope = serde_json::to_value(EventEnvelope::from_emittable(ev))
            .expect("event envelopes always serialize");
        self.deliverer.call(STORE_ON_EVENTS_FN, envelope).await;
    }
}

#[async_trait]
impl EventSink for Emitter {
    fn wants(&self, kind: EventKind) -> bool {
        Emitter::wants(self, kind)
    }

    async fn publish_all(&self, events: &[EmittableEvent]) {
        self.emit_all(events).await;
    }
}

/// Bridge-mode sink: batch the mutation's events into one
/// `session::store::publish-events` call to the main instance.
/// Failures are logged and swallowed (same best-effort stance as the
/// fire-and-forget local calls): the mutation itself already succeeded
/// against the main store.
pub struct RemotePublisher {
    remote: Arc<IIIClient>,
    timeout_ms: u64,
}

impl RemotePublisher {
    pub fn new(remote: Arc<IIIClient>, timeout_ms: u64) -> Self {
        Self { remote, timeout_ms }
    }
}

#[async_trait]
impl EventSink for RemotePublisher {
    /// Always: the main instance's observers and every bridged instance's
    /// observers are out of sight here, and the main's [`Emitter`] gates
    /// each call on them.
    fn wants(&self, _kind: EventKind) -> bool {
        true
    }

    async fn publish_all(&self, events: &[EmittableEvent]) {
        if events.is_empty() {
            return;
        }
        let envelopes: Vec<EventEnvelope> =
            events.iter().map(EventEnvelope::from_emittable).collect();
        let res = self
            .remote
            .call(CallRequest {
                function_id: crate::functions::store_protocol::PUBLISH_EVENTS.to_string(),
                payload: json!({ "events": envelopes }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await;
        if let Err(e) = res {
            tracing::warn!(
                error = %e,
                count = envelopes.len(),
                "failed to publish events to the main instance; local observers will miss them"
            );
        }
    }
}

/// A bridged instance's relay: the function it registered on the main's
/// engine and its observer of the main's [`STORE_ON_EVENTS_FN`].
pub struct BridgeRelay {
    pub function_id: String,
    pub observer: ObserverHandle,
}

impl BridgeRelay {
    /// Remove the observer, so the main stops delivering to this relay.
    pub async fn detach(&self) {
        if let Err(e) = self.observer.dispose().await {
            tracing::warn!(
                relay = %self.function_id,
                error = %e,
                "failed to dispose the bridge relay observer"
            );
        }
    }
}

/// Bridge boot: register a uniquely-named relay function on the main's
/// engine and observe the main's [`STORE_ON_EVENTS_FN`] with it. The relay
/// handler runs in this process (it owns the remote connection) and emits
/// each envelope through the **local** emitter: it never re-publishes, so
/// there are no echo loops. The observer stays `pending` until the main
/// registers the feed.
pub fn attach_bridge_relay(remote: &Arc<IIIClient>, local_emitter: Arc<Emitter>) -> BridgeRelay {
    let relay_id = format!("session::bridge::recv::{}", uuid::Uuid::new_v4().simple());

    let emitter = local_emitter.clone();
    remote.register_function(
        relay_id.clone(),
        RegisterFunction::new_async(move |envelope: EventEnvelope| {
            let emitter = emitter.clone();
            async move {
                emitter
                    .emit_envelopes(std::slice::from_ref(&envelope))
                    .await;
                Ok::<_, Error>(json!({ "ok": true }))
            }
        })
        .description("Internal: session event relay for one bridged session-manager instance.")
        // One relay invocation per session event on the main bus: hidden by
        // default in trace UIs (workers/docs/sops/trace-hidden-functions.md).
        .metadata(json!({ "trace_hidden": true })),
    );

    let observer = remote.observe(
        Source::function(STORE_ON_EVENTS_FN).deliver(Deliver::Output),
        relay_id.as_str(),
    );
    let relay = relay_id.clone();
    // The listener lives as long as the observer: dropping its handle keeps it.
    let _ = observer.on_state_change(move |state, reason| match state {
        ObserverState::Active => {
            tracing::info!(relay = %relay, "receiving the main instance's session events")
        }
        ObserverState::Error => tracing::warn!(
            relay = %relay,
            code = reason.map(|r| r.code.as_str()).unwrap_or("unknown"),
            message = reason.map(|r| r.message.as_str()).unwrap_or(""),
            "the main instance refused the relay's observer; local observers will not receive events"
        ),
        _ => {}
    });

    BridgeRelay {
        function_id: relay_id,
        observer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::types::Role;

    fn map(v: Value) -> JsonMap {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn function_ids_round_trip() {
        for kind in EventKind::all() {
            assert_eq!(EventKind::from_function_id(kind.function_id()), Some(kind));
        }
        assert_eq!(EventKind::from_function_id("session::created"), None);
        assert_eq!(
            EventKind::all().map(|k| k.function_id()),
            [
                "session::on-created",
                "session::on-message-added",
                "session::on-message-updated",
                "session::on-status-changed",
                "session::on-meta-updated",
                "session::on-deleted",
            ]
        );
    }

    #[test]
    fn payload_carries_session_metadata_when_the_session_has_any() {
        let with = EmittableEvent {
            event: SessionEvent::Deleted(SessionDeletedEvent {
                session_id: "s_1".into(),
                timestamp: 5,
            }),
            session_metadata: Some(map(json!({ "owner": "u_1" }))),
        };
        assert_eq!(
            with.payload(),
            json!({ "session_id": "s_1", "timestamp": 5, "session_metadata": { "owner": "u_1" } })
        );

        let without = EmittableEvent {
            session_metadata: None,
            ..with
        };
        assert_eq!(
            without.payload(),
            json!({ "session_id": "s_1", "timestamp": 5 })
        );
    }

    /// The handler's typed round trip (what observers observe) keeps every
    /// field of the payload the emitter sends.
    #[test]
    fn with_session_metadata_round_trips_the_payload() {
        let ev = EmittableEvent {
            event: SessionEvent::MessageAdded(MessageAddedEvent {
                session_id: "s_1".into(),
                entry_id: "e_1".into(),
                parent_id: None,
                message: Some(AgentMessage::User {
                    content: vec![],
                    timestamp: 1,
                }),
                custom: None,
                origin: Some(map(json!({ "turn_id": "t_1" }))),
                timestamp: 2,
            }),
            session_metadata: Some(map(json!({ "owner": "u_1" }))),
        };
        let payload = ev.payload();
        assert_eq!(payload["message"]["role"], json!(Role::User));
        let typed: WithSessionMetadata<MessageAddedEvent> =
            serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), payload);
    }

    #[test]
    fn envelope_roundtrip_preserves_event_and_metadata() {
        let events = vec![
            EmittableEvent {
                event: SessionEvent::Created(SessionCreatedEvent {
                    session_id: "s_1".into(),
                    title: "t".into(),
                    description: "d".into(),
                    status: SessionStatus::Idle,
                    forked_from: Some("s_0".into()),
                    created_at: 7,
                }),
                session_metadata: Some(map(json!({ "owner": "u_1" }))),
            },
            EmittableEvent {
                event: SessionEvent::MessageUpdated(MessageUpdatedEvent {
                    session_id: "s_1".into(),
                    entry_id: "e_1".into(),
                    message: AgentMessage::User {
                        content: vec![],
                        timestamp: 1,
                    },
                    revision: 3,
                    origin: Some(map(json!({ "turn_id": "t_1" }))),
                    timestamp: 9,
                }),
                session_metadata: None,
            },
            EmittableEvent {
                event: SessionEvent::Deleted(SessionDeletedEvent {
                    session_id: "s_1".into(),
                    timestamp: 11,
                }),
                session_metadata: Some(map(json!({ "owner": "u_1" }))),
            },
        ];
        for original in events {
            let envelope = EventEnvelope::from_emittable(&original);
            // The envelope survives JSON (what actually crosses the bus).
            let wire = serde_json::to_value(&envelope).unwrap();
            let back: EventEnvelope = serde_json::from_value(wire).unwrap();
            let restored = back.to_emittable().unwrap();
            assert_eq!(restored, original);
        }
    }

    #[test]
    fn envelope_rejects_unknown_function_and_malformed_payload() {
        let bad_function = EventEnvelope {
            function_id: "session::bogus".into(),
            payload: json!({}),
            session_metadata: None,
        };
        assert!(bad_function
            .to_emittable()
            .unwrap_err()
            .contains("unknown event function"));

        let bad_payload = EventEnvelope {
            function_id: ON_DELETED_FN.into(),
            payload: json!({ "nope": true }),
            session_metadata: None,
        };
        assert!(bad_payload
            .to_emittable()
            .unwrap_err()
            .contains("malformed session::on-deleted envelope payload"));
    }

    /// Records every call for emission assertions; `observed` lists the
    /// functions that have an observer.
    struct TestDeliverer {
        observed: Vec<&'static str>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl TestDeliverer {
        fn observing(observed: &[&'static str]) -> Arc<Self> {
            Arc::new(Self {
                observed: observed.to_vec(),
                calls: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl EventDeliverer for TestDeliverer {
        fn is_observed(&self, function_id: &str) -> bool {
            self.observed.contains(&function_id)
        }

        async fn call(&self, function_id: &str, payload: Value) {
            self.calls
                .lock()
                .unwrap()
                .push((function_id.to_string(), payload));
        }
    }

    fn deleted(session_metadata: Option<JsonMap>) -> EmittableEvent {
        EmittableEvent {
            event: SessionEvent::Deleted(SessionDeletedEvent {
                session_id: "s_1".into(),
                timestamp: 1,
            }),
            session_metadata,
        }
    }

    #[tokio::test]
    async fn an_unobserved_function_is_never_called() {
        let deliverer = TestDeliverer::observing(&[ON_CREATED_FN, STORE_ON_EVENTS_FN]);
        let emitter = Emitter::new(deliverer.clone());
        emitter.emit_all(&[deleted(None)]).await;
        assert!(deliverer.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn emit_all_calls_the_event_function_with_the_session_metadata() {
        let deliverer = TestDeliverer::observing(&[ON_DELETED_FN]);
        let emitter = Emitter::new(deliverer.clone());
        emitter
            .emit_all(&[deleted(Some(map(json!({ "owner": "u_1" }))))])
            .await;
        let calls = deliverer.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, ON_DELETED_FN);
        assert_eq!(calls[0].1["session_id"], "s_1");
        assert_eq!(calls[0].1["session_metadata"]["owner"], "u_1");
    }

    #[tokio::test]
    async fn the_main_emitter_feeds_bridges_once_per_event() {
        let deliverer = TestDeliverer::observing(&[STORE_ON_EVENTS_FN]);
        let emitter = Emitter::feeding_bridges(deliverer.clone());
        emitter
            .emit_all(&[deleted(Some(map(json!({ "owner": "u_1" }))))])
            .await;

        let calls = deliverer.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "one envelope, however many bridges observe");
        let (function_id, envelope) = &calls[0];
        assert_eq!(function_id, STORE_ON_EVENTS_FN);
        assert_eq!(envelope["function_id"], ON_DELETED_FN);
        assert_eq!(envelope["payload"]["session_id"], "s_1");
        assert!(envelope["payload"].get("session_metadata").is_none());
        assert_eq!(envelope["session_metadata"]["owner"], "u_1");
    }

    #[tokio::test]
    async fn a_bridged_emitter_never_calls_the_feed() {
        let deliverer = TestDeliverer::observing(&[STORE_ON_EVENTS_FN]);
        let emitter = Emitter::new(deliverer.clone());
        emitter.emit_all(&[deleted(None)]).await;
        assert!(deliverer.calls.lock().unwrap().is_empty());
    }

    /// `wants` answers what `emit_all` would call: the kind's own function,
    /// or (main mode only) the bridge feed.
    #[test]
    fn an_emitter_wants_an_event_only_while_something_would_receive_it() {
        let nothing = TestDeliverer::observing(&[]);
        assert!(!Emitter::new(nothing.clone()).wants(EventKind::MessageUpdated));
        assert!(!Emitter::feeding_bridges(nothing).wants(EventKind::MessageUpdated));

        let own = TestDeliverer::observing(&[ON_MESSAGE_UPDATED_FN]);
        let emitter = Emitter::new(own);
        assert!(emitter.wants(EventKind::MessageUpdated));
        assert!(!emitter.wants(EventKind::MessageAdded));

        let feed = TestDeliverer::observing(&[STORE_ON_EVENTS_FN]);
        assert!(Emitter::feeding_bridges(feed.clone()).wants(EventKind::MessageUpdated));
        assert!(
            !Emitter::new(feed).wants(EventKind::MessageUpdated),
            "a bridged instance's emitter never calls the feed"
        );
    }

    /// A bridged instance sends every event to the main, whose emitter gates
    /// each call on the observers it sees.
    #[test]
    fn a_remote_publisher_wants_every_event() {
        // register_worker returns at once; nothing connects for this check.
        let remote = Arc::new(iii_sdk::register_worker(
            "ws://127.0.0.1:59625",
            iii_sdk::InitOptions::default(),
        ));
        let publisher = RemotePublisher::new(remote.clone(), 1_000);
        for kind in EventKind::all() {
            assert!(EventSink::wants(&publisher, kind));
        }
        remote.shutdown();
    }

    #[tokio::test]
    async fn emit_envelopes_skips_malformed_and_counts_accepted() {
        let deliverer = TestDeliverer::observing(&[ON_DELETED_FN]);
        let emitter = Emitter::new(deliverer.clone());

        let good = EventEnvelope {
            function_id: ON_DELETED_FN.into(),
            payload: json!({ "session_id": "s_1", "timestamp": 5 }),
            session_metadata: None,
        };
        let bad = EventEnvelope {
            function_id: "session::bogus".into(),
            payload: json!({}),
            session_metadata: None,
        };
        let accepted = emitter.emit_envelopes(&[good, bad]).await;
        assert_eq!(accepted, 1);
        let calls = deliverer.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, ON_DELETED_FN);
    }
}
