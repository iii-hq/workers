//! The six custom trigger types this worker emits, and the fan-out
//! machinery behind them.
//!
//! Reactivity model (session-manager.md § Reactivity model): there is
//! no "subscribe" function — consumers bind handlers to these trigger
//! types with the standard two-step pattern and the engine routes each
//! registration to our [`TriggerHandler`]s. Every mutation emits an
//! event; delivery is fire-and-forget (`TriggerAction::Void`),
//! at-least-once, and unordered — consumers reconcile via `revision`
//! and the parent chain, never via arrival order.
//!
//! Per-binding `config` filters are evaluated **here**, by the emitting
//! worker: `session_id` equality, `roles` membership (message events),
//! and `metadata` subset-equality against `SessionMeta.metadata` (the
//! tenancy hook). Malformed configs are rejected at registration time
//! so a typo'd binding fails loudly instead of silently receiving
//! nothing.
//!
//! After a worker restart the engine replays every existing trigger
//! registration of a re-registered type back to its owner, so the
//! subscriber sets rebuild themselves.
//!
//! Cross-instance propagation (bridge topology): the **main** (fs-mode)
//! instance is the single fan-out point. It additionally registers the
//! internal trigger type [`STORE_EVENTS`] whose payload is an
//! [`EventEnvelope`]; every bridged instance subscribes a relay to it
//! at boot ([`attach_bridge_relay`]). A bridged instance never emits
//! locally at mutation time — its [`RemotePublisher`] sends the
//! envelopes to the main (`session::store::publish_events`), the main
//! runs them through its own [`Emitter`] (local subscribers + envelope
//! fan-out to *all* bridges, originator included), and each bridge's
//! relay re-emits through its local `Emitter`. One canonical path, no
//! dedup needed, and filters are evaluated at every edge because the
//! envelope carries the session metadata.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::{
    IIIError, RegisterTriggerType, TriggerAction, TriggerConfig, TriggerHandler, TriggerRequest,
    III,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{metadata_matches, AgentMessage, CustomPayload, JsonMap, Role, SessionStatus};

pub const CREATED: &str = "session::created";
pub const MESSAGE_ADDED: &str = "session::message-added";
pub const MESSAGE_UPDATED: &str = "session::message-updated";
pub const STATUS_CHANGED: &str = "session::status-changed";
pub const META_UPDATED: &str = "session::meta-updated";
pub const DELETED: &str = "session::deleted";

/// Internal trigger type: envelope feed for bridged instances. Served
/// only by the main (fs-mode) instance.
pub const STORE_EVENTS: &str = "session::store::events";

/// Which of the six trigger types an event belongs to.
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
    pub fn trigger_type(&self) -> &'static str {
        match self {
            EventKind::Created => CREATED,
            EventKind::MessageAdded => MESSAGE_ADDED,
            EventKind::MessageUpdated => MESSAGE_UPDATED,
            EventKind::StatusChanged => STATUS_CHANGED,
            EventKind::MetaUpdated => META_UPDATED,
            EventKind::Deleted => DELETED,
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

    pub fn from_trigger_type(t: &str) -> Option<EventKind> {
        EventKind::all().into_iter().find(|k| k.trigger_type() == t)
    }
}

// ---------------------------------------------------------------------------
// Binding configs (what a subscriber passes in `registerTrigger.config`)
// ---------------------------------------------------------------------------

/// Config accepted by `session::created` bindings. The spec defines no
/// filters for this type beyond the universal `metadata` match.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatedBindingConfig {
    /// Equality match against `SessionMeta.metadata`: every key given
    /// here must equal the stored value (subset match).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonMap>,
}

/// Config accepted by `session::status-changed`, `session::meta-updated`
/// and `session::deleted` bindings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionBindingConfig {
    /// Only deliver events for this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Equality match against `SessionMeta.metadata` (subset match).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonMap>,
}

/// Config accepted by `session::message-added` and
/// `session::message-updated` bindings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageBindingConfig {
    /// Only deliver events for this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Only deliver events whose message role is in this list.
    /// `kind: "custom"` entries carry no role and never match a
    /// `roles` filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<Role>>,
    /// Equality match against `SessionMeta.metadata` (subset match).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonMap>,
}

/// A parsed binding filter — the normalized form of any of the three
/// config shapes above.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BindingFilter {
    pub session_id: Option<String>,
    pub roles: Option<Vec<Role>>,
    pub metadata: Option<JsonMap>,
}

impl EventKind {
    /// Parse and validate a raw binding config for this trigger type.
    /// `null` is treated as `{}`. Unknown fields are rejected so a
    /// misspelled filter key fails at registration, not silently.
    pub fn parse_binding_config(&self, raw: &Value) -> Result<BindingFilter, String> {
        let raw = if raw.is_null() {
            &Value::Object(serde_json::Map::new())
        } else {
            raw
        };
        let type_name = self.trigger_type();
        match self {
            EventKind::Created => {
                let c: CreatedBindingConfig = serde_json::from_value(raw.clone())
                    .map_err(|e| format!("invalid {type_name} config: {e}"))?;
                Ok(BindingFilter {
                    session_id: None,
                    roles: None,
                    metadata: c.metadata,
                })
            }
            EventKind::MessageAdded | EventKind::MessageUpdated => {
                let c: MessageBindingConfig = serde_json::from_value(raw.clone())
                    .map_err(|e| format!("invalid {type_name} config: {e}"))?;
                Ok(BindingFilter {
                    session_id: c.session_id,
                    roles: c.roles,
                    metadata: c.metadata,
                })
            }
            EventKind::StatusChanged | EventKind::MetaUpdated | EventKind::Deleted => {
                let c: SessionBindingConfig = serde_json::from_value(raw.clone())
                    .map_err(|e| format!("invalid {type_name} config: {e}"))?;
                Ok(BindingFilter {
                    session_id: c.session_id,
                    roles: None,
                    metadata: c.metadata,
                })
            }
        }
    }
}

/// The event-side facts a filter is evaluated against.
#[derive(Debug, Clone, Copy)]
pub struct EventCtx<'a> {
    pub session_id: &'a str,
    /// Role of the message the event is about (message events on
    /// `kind: "message"` entries only).
    pub role: Option<Role>,
    /// The session's `metadata` as of the mutation (for `session::deleted`,
    /// as of just before deletion).
    pub session_metadata: Option<&'a JsonMap>,
}

/// Pure filter evaluation: does this binding receive this event?
pub fn binding_matches(filter: &BindingFilter, ctx: &EventCtx<'_>) -> bool {
    if let Some(want_sid) = &filter.session_id {
        if want_sid != ctx.session_id {
            return false;
        }
    }
    if let Some(roles) = &filter.roles {
        match ctx.role {
            Some(role) if roles.contains(&role) => {}
            _ => return false,
        }
    }
    if let Some(want) = &filter.metadata {
        if !metadata_matches(want, ctx.session_metadata) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Event payloads (what subscribers receive)
// ---------------------------------------------------------------------------

/// `session::created` — a new session exists (create or fork).
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

/// `session::message-added` — an entry was appended. Carries `message`
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

/// `session::message-updated` — a message's content changed.
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

/// `session::status-changed` — a session's status changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatusChangedEvent {
    pub session_id: String,
    pub status: SessionStatus,
    pub previous_status: SessionStatus,
    /// Short cause, set on `"error"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    pub timestamp: i64,
}

/// `session::meta-updated` — title/description/metadata changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MetaUpdatedEvent {
    pub session_id: String,
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonMap>,
    pub timestamp: i64,
}

/// `session::deleted` — a session and its entries were removed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionDeletedEvent {
    pub session_id: String,
    pub timestamp: i64,
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

    /// The role a `roles` filter is matched against.
    pub fn role(&self) -> Option<Role> {
        match self {
            SessionEvent::MessageAdded(e) => e.message.as_ref().map(|m| m.role()),
            SessionEvent::MessageUpdated(e) => Some(e.message.role()),
            _ => None,
        }
    }

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

/// An event paired with the session metadata snapshot its `metadata`
/// filters are evaluated against.
#[derive(Debug, Clone, PartialEq)]
pub struct EmittableEvent {
    pub event: SessionEvent,
    pub session_metadata: Option<JsonMap>,
}

/// Wire form of an [`EmittableEvent`] for cross-instance propagation.
/// `session_metadata` rides along so `metadata` filters keep working
/// at every edge (the spec payloads do not carry it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventEnvelope {
    /// One of the six public trigger type ids.
    pub trigger_type: String,
    /// The spec payload for that trigger type.
    pub payload: Value,
    /// `SessionMeta.metadata` as of the mutation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_metadata: Option<JsonMap>,
}

impl EventEnvelope {
    pub fn from_emittable(ev: &EmittableEvent) -> Self {
        Self {
            trigger_type: ev.event.kind().trigger_type().to_string(),
            payload: ev.event.payload(),
            session_metadata: ev.session_metadata.clone(),
        }
    }

    /// Lossless inverse of [`Self::from_emittable`]. Errors on unknown
    /// trigger types or payloads that do not match the type's shape.
    pub fn to_emittable(&self) -> Result<EmittableEvent, String> {
        let kind = EventKind::from_trigger_type(&self.trigger_type)
            .ok_or_else(|| format!("unknown trigger type {}", self.trigger_type))?;
        let payload = self.payload.clone();
        let parse_err =
            |e: serde_json::Error| format!("malformed {} envelope payload: {e}", self.trigger_type);
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
// Subscriber registry + trigger type registration
// ---------------------------------------------------------------------------

/// One subscriber binding of a trigger type.
#[derive(Debug, Clone)]
pub struct Binding {
    pub id: String,
    pub function_id: String,
    pub filter: BindingFilter,
}

/// Thread-safe subscriber registry for one trigger type, keyed by
/// trigger-instance id. Cloned into both the [`TriggerHandler`]
/// (mutates on register/unregister) and the [`Emitter`] (iterates
/// read-only snapshots).
#[derive(Clone)]
pub struct SubscriberSet {
    kind: EventKind,
    inner: Arc<Mutex<HashMap<String, Binding>>>,
}

impl SubscriberSet {
    pub fn new(kind: EventKind) -> Self {
        Self {
            kind,
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn kind(&self) -> EventKind {
        self.kind
    }

    /// Parse + insert a binding. Rejects malformed configs.
    pub fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let filter = self.kind.parse_binding_config(&config.config)?;
        let binding = Binding {
            id: config.id.clone(),
            function_id: config.function_id,
            filter,
        };
        self.lock().insert(config.id, binding);
        Ok(())
    }

    pub fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    /// Snapshot so the mutex isn't held across awaits.
    pub fn snapshot(&self) -> Vec<Binding> {
        self.lock().values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Binding>> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// The six subscriber sets, one per trigger type.
#[derive(Clone)]
pub struct TriggerSets {
    pub created: SubscriberSet,
    pub message_added: SubscriberSet,
    pub message_updated: SubscriberSet,
    pub status_changed: SubscriberSet,
    pub meta_updated: SubscriberSet,
    pub deleted: SubscriberSet,
}

impl TriggerSets {
    pub fn new() -> Self {
        Self {
            created: SubscriberSet::new(EventKind::Created),
            message_added: SubscriberSet::new(EventKind::MessageAdded),
            message_updated: SubscriberSet::new(EventKind::MessageUpdated),
            status_changed: SubscriberSet::new(EventKind::StatusChanged),
            meta_updated: SubscriberSet::new(EventKind::MetaUpdated),
            deleted: SubscriberSet::new(EventKind::Deleted),
        }
    }

    pub fn for_kind(&self, kind: EventKind) -> &SubscriberSet {
        match kind {
            EventKind::Created => &self.created,
            EventKind::MessageAdded => &self.message_added,
            EventKind::MessageUpdated => &self.message_updated,
            EventKind::StatusChanged => &self.status_changed,
            EventKind::MetaUpdated => &self.meta_updated,
            EventKind::Deleted => &self.deleted,
        }
    }
}

impl Default for TriggerSets {
    fn default() -> Self {
        Self::new()
    }
}

struct SessionTriggerHandler {
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for SessionTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        let kind = self.set.kind();
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        self.set.add(config).map_err(IIIError::Handler)?;
        tracing::info!(
            trigger_type = kind.trigger_type(),
            id = %id,
            function_id = %function_id,
            "trigger subscription registered"
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        tracing::info!(
            trigger_type = self.set.kind().trigger_type(),
            id = %config.id,
            "trigger subscription unregistered"
        );
        self.set.remove(&config.id);
        Ok(())
    }
}

/// Unfiltered subscriber registry for the internal [`STORE_EVENTS`]
/// trigger type: binding id -> relay function id. One subscriber per
/// attached bridged instance.
#[derive(Clone, Default)]
pub struct BridgeSubscribers {
    inner: Arc<Mutex<HashMap<String, String>>>,
}

impl BridgeSubscribers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&self, id: String, function_id: String) {
        self.lock().insert(id, function_id);
    }

    pub fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    /// Snapshot so the mutex isn't held across awaits.
    pub fn function_ids(&self) -> Vec<String> {
        self.lock().values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// Config accepted by [`STORE_EVENTS`] bindings: none. The feed is
/// unfiltered by design — each bridged instance filters locally.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct StoreEventsBindingConfig {}

struct StoreEventsTriggerHandler {
    subscribers: BridgeSubscribers,
}

#[async_trait]
impl TriggerHandler for StoreEventsTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        tracing::info!(
            trigger_type = STORE_EVENTS,
            id = %config.id,
            function_id = %config.function_id,
            "bridge relay attached"
        );
        self.subscribers.add(config.id, config.function_id);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        tracing::info!(
            trigger_type = STORE_EVENTS,
            id = %config.id,
            "bridge relay detached"
        );
        self.subscribers.remove(&config.id);
        Ok(())
    }
}

/// Register the internal [`STORE_EVENTS`] trigger type (main/fs mode
/// only). The engine replays existing registrations on re-register, so
/// attached bridges survive a main restart.
pub fn register_store_events_type(iii: &Arc<III>) -> BridgeSubscribers {
    let subscribers = BridgeSubscribers::new();
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            STORE_EVENTS,
            "Internal: event-envelope feed for bridged session-manager instances. \
             Not for direct consumption — bind the six public session::* types instead.",
            StoreEventsTriggerHandler {
                subscribers: subscribers.clone(),
            },
        )
        .trigger_request_format::<StoreEventsBindingConfig>(),
    );
    tracing::info!(trigger_type = STORE_EVENTS, "registered trigger type");
    subscribers
}

/// Register the six custom trigger types with the engine. Must run
/// **before** `functions::register_all` so handlers can capture the
/// subscriber sets.
pub fn register_trigger_types(iii: &Arc<III>) -> TriggerSets {
    let sets = TriggerSets::new();

    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            CREATED,
            "A new session exists (via session::create or session::fork).",
            SessionTriggerHandler {
                set: sets.created.clone(),
            },
        )
        .trigger_request_format::<CreatedBindingConfig>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            MESSAGE_ADDED,
            "A message entry was appended to a session.",
            SessionTriggerHandler {
                set: sets.message_added.clone(),
            },
        )
        .trigger_request_format::<MessageBindingConfig>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            MESSAGE_UPDATED,
            "A message entry's content changed (e.g. streaming deltas).",
            SessionTriggerHandler {
                set: sets.message_updated.clone(),
            },
        )
        .trigger_request_format::<MessageBindingConfig>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            STATUS_CHANGED,
            "A session's status changed (idle/working/done/error).",
            SessionTriggerHandler {
                set: sets.status_changed.clone(),
            },
        )
        .trigger_request_format::<SessionBindingConfig>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            META_UPDATED,
            "A session's title/description/metadata changed.",
            SessionTriggerHandler {
                set: sets.meta_updated.clone(),
            },
        )
        .trigger_request_format::<SessionBindingConfig>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            DELETED,
            "A session and its entries were removed.",
            SessionTriggerHandler {
                set: sets.deleted.clone(),
            },
        )
        .trigger_request_format::<SessionBindingConfig>(),
    );

    for kind in EventKind::all() {
        tracing::info!(
            trigger_type = kind.trigger_type(),
            "registered trigger type"
        );
    }

    sets
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// How a matched event reaches a subscriber. Production delivers over
/// the bus; tests record.
#[async_trait]
pub trait EventDeliverer: Send + Sync {
    async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value);
}

/// Fire-and-forget bus delivery (`TriggerAction::Void`) so the mutation
/// that produced the event is never blocked on subscriber latency.
/// Failures are logged and swallowed — a misbehaving subscriber must
/// not break the write path.
pub struct IiiDeliverer {
    iii: Arc<III>,
}

impl IiiDeliverer {
    pub fn new(iii: Arc<III>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl EventDeliverer for IiiDeliverer {
    async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value) {
        let res = self
            .iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            })
            .await;
        if let Err(e) = res {
            tracing::warn!(
                trigger_type,
                function_id,
                error = %e,
                "event fan-out failed"
            );
        }
    }
}

/// Where a mutation's events go. fs mode publishes through the local
/// [`Emitter`]; bridge mode publishes to the main instance via
/// [`RemotePublisher`].
#[async_trait]
pub trait EventSink: Send + Sync {
    async fn publish_all(&self, events: &[EmittableEvent]);
}

/// Evaluates each binding's filter against each event and delivers the
/// payload to every match; additionally fans the envelope out to every
/// attached bridged instance (main mode — the bridge set is empty on
/// bridged instances, making that path a no-op there).
pub struct Emitter {
    sets: TriggerSets,
    deliverer: Arc<dyn EventDeliverer>,
    bridges: BridgeSubscribers,
}

impl Emitter {
    /// Emitter without bridge fan-out (bridge mode and most tests).
    pub fn new(sets: TriggerSets, deliverer: Arc<dyn EventDeliverer>) -> Self {
        Self::with_bridges(sets, deliverer, BridgeSubscribers::new())
    }

    /// Emitter with bridge fan-out (main/fs mode).
    pub fn with_bridges(
        sets: TriggerSets,
        deliverer: Arc<dyn EventDeliverer>,
        bridges: BridgeSubscribers,
    ) -> Self {
        Self {
            sets,
            deliverer,
            bridges,
        }
    }

    pub fn sets(&self) -> &TriggerSets {
        &self.sets
    }

    pub fn bridges(&self) -> &BridgeSubscribers {
        &self.bridges
    }

    pub async fn emit_all(&self, events: &[EmittableEvent]) {
        for ev in events {
            self.emit(ev).await;
            self.fan_out_to_bridges(ev).await;
        }
    }

    /// Ingest path for externally-produced envelopes (the main's
    /// `session::store::publish_events` and the bridges' relays).
    /// Returns the number of well-formed envelopes processed; malformed
    /// ones are skipped with a warning.
    pub async fn emit_envelopes(&self, envelopes: &[EventEnvelope]) -> usize {
        let mut accepted = 0;
        for envelope in envelopes {
            match envelope.to_emittable() {
                Ok(ev) => {
                    self.emit(&ev).await;
                    self.fan_out_to_bridges(&ev).await;
                    accepted += 1;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "dropping malformed event envelope");
                }
            }
        }
        accepted
    }

    /// Local fan-out: filter evaluation + delivery to matching bindings.
    async fn emit(&self, ev: &EmittableEvent) {
        let kind = ev.event.kind();
        let bindings = self.sets.for_kind(kind).snapshot();
        if bindings.is_empty() {
            return;
        }
        let payload = ev.event.payload();
        let ctx = EventCtx {
            session_id: ev.event.session_id(),
            role: ev.event.role(),
            session_metadata: ev.session_metadata.as_ref(),
        };
        for binding in bindings {
            if binding_matches(&binding.filter, &ctx) {
                self.deliverer
                    .deliver(kind.trigger_type(), &binding.function_id, payload.clone())
                    .await;
            }
        }
    }

    /// Envelope fan-out to attached bridged instances (unfiltered —
    /// each bridge filters locally).
    async fn fan_out_to_bridges(&self, ev: &EmittableEvent) {
        let targets = self.bridges.function_ids();
        if targets.is_empty() {
            return;
        }
        let envelope = serde_json::to_value(EventEnvelope::from_emittable(ev))
            .expect("event envelopes always serialize");
        for function_id in targets {
            self.deliverer
                .deliver(STORE_EVENTS, &function_id, envelope.clone())
                .await;
        }
    }
}

#[async_trait]
impl EventSink for Emitter {
    async fn publish_all(&self, events: &[EmittableEvent]) {
        self.emit_all(events).await;
    }
}

/// Bridge-mode sink: batch the mutation's events into one
/// `session::store::publish_events` call to the main instance.
/// Failures are logged and swallowed (same best-effort stance as the
/// fire-and-forget local fan-out): the mutation itself already
/// succeeded against the main store.
pub struct RemotePublisher {
    remote: Arc<III>,
    timeout_ms: u64,
}

impl RemotePublisher {
    pub fn new(remote: Arc<III>, timeout_ms: u64) -> Self {
        Self { remote, timeout_ms }
    }
}

#[async_trait]
impl EventSink for RemotePublisher {
    async fn publish_all(&self, events: &[EmittableEvent]) {
        if events.is_empty() {
            return;
        }
        let envelopes: Vec<EventEnvelope> =
            events.iter().map(EventEnvelope::from_emittable).collect();
        let res = self
            .remote
            .trigger(TriggerRequest {
                function_id: crate::functions::store_protocol::PUBLISH_EVENTS.to_string(),
                payload: serde_json::json!({ "events": envelopes }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await;
        if let Err(e) = res {
            tracing::warn!(
                error = %e,
                count = envelopes.len(),
                "failed to publish events to the main instance; local subscribers will miss them"
            );
        }
    }
}

/// Bridge boot: register a uniquely-named relay function on the main
/// bus and bind it to the main's [`STORE_EVENTS`] feed. The relay
/// handler runs in this process (it owns the remote connection) and
/// re-emits each envelope through the **local** emitter — it never
/// re-publishes, so there are no echo loops. Returns the relay
/// function id.
pub fn attach_bridge_relay(remote: &Arc<III>, local_emitter: Arc<Emitter>) -> String {
    let relay_id = format!("session::bridge::recv::{}", uuid::Uuid::new_v4().simple());

    let emitter = local_emitter.clone();
    remote.register_function(
        relay_id.clone(),
        iii_sdk::RegisterFunction::new_async(move |envelope: EventEnvelope| {
            let emitter = emitter.clone();
            async move {
                emitter
                    .emit_envelopes(std::slice::from_ref(&envelope))
                    .await;
                Ok::<_, IIIError>(serde_json::json!({ "ok": true }))
            }
        })
        .description("Internal: session event relay for one bridged session-manager instance."),
    );

    match remote.register_trigger(iii_sdk::RegisterTriggerInput {
        trigger_type: STORE_EVENTS.to_string(),
        function_id: relay_id.clone(),
        config: serde_json::json!({}),
        metadata: None,
    }) {
        Ok(_) => tracing::info!(relay = %relay_id, "attached to the main instance's event feed"),
        Err(e) => tracing::warn!(
            error = %e,
            relay = %relay_id,
            "failed to attach to the main instance's event feed; local subscribers will not receive events"
        ),
    }

    relay_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(v: Value) -> JsonMap {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn empty_filter_matches_everything() {
        let f = BindingFilter::default();
        let ctx = EventCtx {
            session_id: "s_1",
            role: Some(Role::Assistant),
            session_metadata: None,
        };
        assert!(binding_matches(&f, &ctx));
    }

    #[test]
    fn session_id_filter() {
        let f = BindingFilter {
            session_id: Some("s_1".into()),
            ..Default::default()
        };
        let mk = |sid: &'static str| EventCtx {
            session_id: sid,
            role: None,
            session_metadata: None,
        };
        assert!(binding_matches(&f, &mk("s_1")));
        assert!(!binding_matches(&f, &mk("s_2")));
    }

    #[test]
    fn roles_filter_requires_a_matching_role() {
        let f = BindingFilter {
            roles: Some(vec![Role::Assistant, Role::User]),
            ..Default::default()
        };
        let mk = |role: Option<Role>| EventCtx {
            session_id: "s_1",
            role,
            session_metadata: None,
        };
        assert!(binding_matches(&f, &mk(Some(Role::Assistant))));
        assert!(binding_matches(&f, &mk(Some(Role::User))));
        assert!(!binding_matches(&f, &mk(Some(Role::FunctionResult))));
        // kind: "custom" entries have no role -> never match a roles filter.
        assert!(!binding_matches(&f, &mk(None)));
    }

    #[test]
    fn metadata_filter_is_subset_equality() {
        let f = BindingFilter {
            metadata: Some(map(json!({ "owner": "u_1" }))),
            ..Default::default()
        };
        let meta_match = map(json!({ "owner": "u_1", "channel": "web" }));
        let meta_wrong = map(json!({ "owner": "u_2" }));

        fn ctx(m: Option<&JsonMap>) -> EventCtx<'_> {
            EventCtx {
                session_id: "s_1",
                role: None,
                session_metadata: m,
            }
        }
        assert!(binding_matches(&f, &ctx(Some(&meta_match))));
        assert!(!binding_matches(&f, &ctx(Some(&meta_wrong))));
        // Session without metadata never matches a non-empty filter.
        assert!(!binding_matches(&f, &ctx(None)));
    }

    #[test]
    fn empty_metadata_filter_is_vacuous() {
        let f = BindingFilter {
            metadata: Some(JsonMap::new()),
            ..Default::default()
        };
        let ctx = EventCtx {
            session_id: "s_1",
            role: None,
            session_metadata: None,
        };
        assert!(binding_matches(&f, &ctx));
    }

    #[test]
    fn metadata_filter_compares_values_deeply() {
        let f = BindingFilter {
            metadata: Some(map(json!({ "tags": ["a", "b"] }))),
            ..Default::default()
        };
        let exact = map(json!({ "tags": ["a", "b"] }));
        let different = map(json!({ "tags": ["a"] }));
        fn ctx(m: &JsonMap) -> EventCtx<'_> {
            EventCtx {
                session_id: "s_1",
                role: None,
                session_metadata: Some(m),
            }
        }
        assert!(binding_matches(&f, &ctx(&exact)));
        assert!(!binding_matches(&f, &ctx(&different)));
    }

    #[test]
    fn parse_rejects_unknown_fields() {
        let err = EventKind::MessageAdded
            .parse_binding_config(&json!({ "sesion_id": "s_1" }))
            .unwrap_err();
        assert!(err.contains("invalid session::message-added config"));
    }

    #[test]
    fn parse_rejects_session_id_on_created() {
        // session::created has no per-session filter in the spec.
        assert!(EventKind::Created
            .parse_binding_config(&json!({ "session_id": "s_1" }))
            .is_err());
    }

    #[test]
    fn parse_rejects_roles_on_status_changed() {
        assert!(EventKind::StatusChanged
            .parse_binding_config(&json!({ "roles": ["assistant"] }))
            .is_err());
    }

    #[test]
    fn parse_rejects_invalid_role_values() {
        assert!(EventKind::MessageAdded
            .parse_binding_config(&json!({ "roles": ["bogus"] }))
            .is_err());
    }

    #[test]
    fn parse_accepts_null_and_empty_configs() {
        assert_eq!(
            EventKind::Deleted
                .parse_binding_config(&Value::Null)
                .unwrap(),
            BindingFilter::default()
        );
        assert_eq!(
            EventKind::Deleted.parse_binding_config(&json!({})).unwrap(),
            BindingFilter::default()
        );
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
    fn envelope_rejects_unknown_type_and_malformed_payload() {
        let bad_type = EventEnvelope {
            trigger_type: "session::bogus".into(),
            payload: json!({}),
            session_metadata: None,
        };
        assert!(bad_type
            .to_emittable()
            .unwrap_err()
            .contains("unknown trigger type"));

        let bad_payload = EventEnvelope {
            trigger_type: DELETED.into(),
            payload: json!({ "nope": true }),
            session_metadata: None,
        };
        assert!(bad_payload
            .to_emittable()
            .unwrap_err()
            .contains("malformed session::deleted envelope payload"));
    }

    /// Records every delivery for fan-out assertions.
    struct TestDeliverer {
        seen: Mutex<Vec<(String, String, Value)>>,
    }

    #[async_trait]
    impl EventDeliverer for TestDeliverer {
        async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value) {
            self.seen.lock().unwrap().push((
                trigger_type.to_string(),
                function_id.to_string(),
                payload,
            ));
        }
    }

    #[tokio::test]
    async fn emit_all_fans_envelopes_to_every_bridge() {
        let deliverer = Arc::new(TestDeliverer {
            seen: Mutex::new(Vec::new()),
        });
        let bridges = BridgeSubscribers::new();
        bridges.add("t1".into(), "session::bridge::recv::aaa".into());
        bridges.add("t2".into(), "session::bridge::recv::bbb".into());
        let emitter = Emitter::with_bridges(TriggerSets::new(), deliverer.clone(), bridges);

        let ev = EmittableEvent {
            event: SessionEvent::Deleted(SessionDeletedEvent {
                session_id: "s_1".into(),
                timestamp: 1,
            }),
            session_metadata: Some(map(json!({ "owner": "u_1" }))),
        };
        emitter.emit_all(std::slice::from_ref(&ev)).await;

        let seen = deliverer.seen.lock().unwrap();
        let to_bridges: Vec<_> = seen.iter().filter(|(t, _, _)| t == STORE_EVENTS).collect();
        assert_eq!(to_bridges.len(), 2, "one envelope per attached bridge");
        for (_, _, payload) in &to_bridges {
            assert_eq!(payload["trigger_type"], DELETED);
            assert_eq!(payload["payload"]["session_id"], "s_1");
            assert_eq!(payload["session_metadata"]["owner"], "u_1");
        }
    }

    #[tokio::test]
    async fn emit_envelopes_skips_malformed_and_counts_accepted() {
        let deliverer = Arc::new(TestDeliverer {
            seen: Mutex::new(Vec::new()),
        });
        let emitter = Emitter::new(TriggerSets::new(), deliverer.clone());
        emitter
            .sets()
            .for_kind(EventKind::Deleted)
            .add(TriggerConfig {
                id: "b1".into(),
                function_id: "ui::recv".into(),
                config: json!({}),
                metadata: None,
            })
            .unwrap();

        let good = EventEnvelope {
            trigger_type: DELETED.into(),
            payload: json!({ "session_id": "s_1", "timestamp": 5 }),
            session_metadata: None,
        };
        let bad = EventEnvelope {
            trigger_type: "session::bogus".into(),
            payload: json!({}),
            session_metadata: None,
        };
        let accepted = emitter.emit_envelopes(&[good, bad]).await;
        assert_eq!(accepted, 1);
        let seen = deliverer.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].1, "ui::recv");
    }

    #[test]
    fn subscriber_set_add_remove_and_reject() {
        let set = SubscriberSet::new(EventKind::MessageAdded);
        set.add(TriggerConfig {
            id: "t1".into(),
            function_id: "ui::recv".into(),
            config: json!({ "session_id": "s_1", "roles": ["assistant"] }),
            metadata: None,
        })
        .unwrap();
        assert_eq!(set.len(), 1);

        let err = set.add(TriggerConfig {
            id: "t2".into(),
            function_id: "ui::recv".into(),
            config: json!({ "bogus": true }),
            metadata: None,
        });
        assert!(err.is_err());
        assert_eq!(set.len(), 1);

        set.remove("t1");
        assert!(set.is_empty());
    }
}
