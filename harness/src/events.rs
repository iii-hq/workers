//! The async orchestration trigger types the harness emits —
//! `harness::ready` after boot, `harness::turn-started` /
//! `harness::turn-completed` at turn boundaries, `harness::message-queued`
//! when a message parks in the mid-turn queue, `harness::triggers-changed`
//! when a session's binding set or fire count changes, and
//! `harness::compaction` when an explicit compaction starts and finishes
//! (harness.md § Trigger types emitted). Consumers and siblings bind
//! these to react without polling.
//!
//! Delivery is fire-and-forget (`TriggerAction::Void`), at-least-once, and
//! unordered. Per-binding `config` filters (`session_id`, `parent_session_id`)
//! are evaluated here by the emitting worker. The engine replays existing
//! registrations after a restart, so the subscriber sets rebuild themselves.
//!
//! Next to each trigger type is an owner-delivered observable function
//! (iii 0.25): `harness::on-ready`, `harness::on-turn-started`,
//! `harness::on-turn-completed`, `harness::on-message-queued`,
//! `harness::on-triggers-changed` and `harness::on-compaction`. Its args are
//! the trigger config; its [`ObserveHandler`] files observers in the same
//! subscriber set, and every event a binding receives is handed to each
//! matching observer with `obs.call` (see [`crate::observe`]). Invoking an
//! on-* function is refused with FORBIDDEN.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    IIIClient, ObserveHandler, ObserveRejection, Observer, RegisterTriggerType, TriggerAction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::observe::{self, codes, EventObserver};
use crate::types::turn::ParentLink;

pub const TURN_STARTED: &str = "harness::turn-started";
pub const TURN_COMPLETED: &str = "harness::turn-completed";
pub const MESSAGE_QUEUED: &str = "harness::message-queued";
pub const TRIGGERS_CHANGED: &str = "harness::triggers-changed";
pub const READY: &str = "harness::ready";
pub const COMPACTION: &str = "harness::compaction";

/// The observable function next to each trigger type above.
pub const ON_READY: &str = "harness::on-ready";
pub const ON_TURN_STARTED: &str = "harness::on-turn-started";
pub const ON_TURN_COMPLETED: &str = "harness::on-turn-completed";
pub const ON_MESSAGE_QUEUED: &str = "harness::on-message-queued";
pub const ON_TRIGGERS_CHANGED: &str = "harness::on-triggers-changed";
pub const ON_COMPACTION: &str = "harness::on-compaction";

const ON_READY_DESC: &str = "The harness completed boot and can accept turns. Observe it with \
    args {}; each observer receives { status: ready, timestamp }, at once when the harness is \
    already ready. Only the harness emits it: invoking it directly is refused.";
const ON_TURN_STARTED_DESC: &str = "A harness turn began executing (first loop step). Observe it \
    with args { session_id?, parent_session_id? }; each observer receives the \
    harness::turn-started payload { session_id, turn_id, timestamp, depth, message_preview?, \
    parent?, parent_session_id? }. Only the harness emits it: invoking it directly is refused.";
const ON_TURN_COMPLETED_DESC: &str = "A harness turn reached a terminal status \
    (completed/cancelled/failed). Observe it with args { session_id?, parent_session_id? }; each \
    observer receives the harness::turn-completed payload { session_id, turn_id, status, \
    terminal, timestamp, result?, result_error?, reason?, parent?, parent_session_id?, context? \
    }. Only the harness emits it: invoking it directly is refused.";
const ON_MESSAGE_QUEUED_DESC: &str = "A message parked in a session's server-side queue while its \
    turn streams. Observe it with args { session_id?, parent_session_id? }; each observer \
    receives { session_id, entry_id, queued_at, timestamp }. Only the harness emits it: invoking \
    it directly is refused.";
const ON_TRIGGERS_CHANGED_DESC: &str = "A session's trigger-binding set or fire count changed \
    (refetch harness::triggers::list). Observe it with args { session_id?, parent_session_id? }; \
    each observer receives { session_id, timestamp }. Only the harness emits it: invoking it \
    directly is refused.";
const ON_COMPACTION_DESC: &str = "An explicit compaction of a session started (phase: started) \
    or finished (phase: completed, with status ok/busy/empty/failed and the user-facing \
    message). Observe it with args { session_id?, parent_session_id? }; each observer receives \
    the harness::compaction payload. Only the harness emits it: invoking it directly is refused.";

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadyBindingConfig {}

/// Binding config shared by both turn-event types.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnEventBindingConfig {
    /// Only deliver events for this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Only deliver sub-agent events whose parent is this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
}
/// What an observer of `harness::on-ready` (and a `harness::ready` binding)
/// receives.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadyEvent {
    /// Always `ready`.
    pub status: String,
    /// Emission time, epoch milliseconds.
    pub timestamp: i64,
}

/// What an observer of `harness::on-turn-started` (and a
/// `harness::turn-started` binding) receives.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TurnStartedEvent {
    pub session_id: String,
    pub turn_id: String,
    /// Emission time, epoch milliseconds.
    pub timestamp: i64,
    /// Sub-agent depth; 0 for a top-level turn.
    pub depth: u32,
    /// The first characters of the message that started the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_preview: Option<String>,
    /// Sub-agent linkage of a spawned child turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentLink>,
    /// Display parent of a trigger-fired child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
}

/// What an observer of `harness::on-turn-completed` (and a
/// `harness::turn-completed` binding) receives.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TurnCompletedEvent {
    pub session_id: String,
    pub turn_id: String,
    /// `completed`, `cancelled` or `failed`.
    pub status: String,
    /// False while the session still owns an armed wake: progress, not the
    /// run's final outcome.
    pub terminal: bool,
    /// Emission time, epoch milliseconds.
    pub timestamp: i64,
    /// The structured result, when the turn produced one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// The latest generation's context accounting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<crate::context_snapshot::ContextSnapshotV1>,
}

/// What an observer of `harness::on-message-queued` (and a
/// `harness::message-queued` binding) receives: a pointer, not the message.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MessageQueuedEvent {
    pub session_id: String,
    pub entry_id: String,
    /// When the message parked, epoch milliseconds.
    pub queued_at: i64,
    /// Emission time, epoch milliseconds.
    pub timestamp: i64,
}

/// What an observer of `harness::on-triggers-changed` (and a
/// `harness::triggers-changed` binding) receives: a doorbell to refetch
/// `harness::triggers::list`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TriggersChangedEvent {
    pub session_id: String,
    /// Emission time, epoch milliseconds.
    pub timestamp: i64,
}

/// What an observer of `harness::on-compaction` (and a `harness::compaction`
/// binding) receives (`crate::compaction::event_payload`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompactionEvent {
    pub session_id: String,
    pub compaction_id: String,
    /// `started` or `completed`.
    pub phase: String,
    /// Emission time, epoch milliseconds.
    pub timestamp: i64,
    /// On `completed` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<crate::compaction::Status>,
    /// On `completed` only: the outcome as every client shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// On `completed` only: the transcript rows it wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_before: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_after: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BindingFilter {
    pub session_id: Option<String>,
    pub parent_session_id: Option<String>,
}

impl BindingFilter {
    fn parse(raw: &Value) -> Result<Self, String> {
        let raw = if raw.is_null() {
            Value::Object(Default::default())
        } else {
            raw.clone()
        };
        let cfg: TurnEventBindingConfig =
            serde_json::from_value(raw).map_err(|e| format!("invalid turn-event config: {e}"))?;
        Ok(BindingFilter {
            session_id: cfg.session_id,
            parent_session_id: cfg.parent_session_id,
        })
    }

    fn matches(
        &self,
        session_id: &str,
        parent: Option<&ParentLink>,
        display_parent: Option<&str>,
    ) -> bool {
        if let Some(sid) = &self.session_id {
            if sid != session_id {
                return false;
            }
        }
        if let Some(psid) = &self.parent_session_id {
            let link_matches = matches!(parent, Some(p) if &p.session_id == psid);
            let display_matches = display_parent == Some(psid.as_str());
            if !link_matches && !display_matches {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone)]
struct Binding {
    function_id: String,
    namespace: Option<String>,
    filter: BindingFilter,
    /// The trigger's registration `metadata`, forwarded to the bound function
    /// as the invocation sidecar so targets like `harness::spawn` and
    /// `harness::notify_agent` can carry per-subscription context (the spawn
    /// spec, or the subscription id/label). Stamped once at registration and
    /// delivered as stored — there is no fire-time stamping. Carries the
    /// `__subscription_id` the self-edge check in `fan_out` matches on — the
    /// engine's own registration id (`TriggerConfig.id`) is keyed on in
    /// `SubscriberSet` but otherwise unused here.
    metadata: Option<Value>,
}

/// One observer of a `harness::on-*` function with its parsed args.
#[derive(Clone)]
struct ObserverEntry {
    observer: Arc<dyn EventObserver>,
    filter: BindingFilter,
}

/// One trigger type's bindings and the observers of its `harness::on-*`
/// function, kept apart by id.
#[derive(Clone, Default)]
pub struct SubscriberSet {
    inner: Arc<Mutex<HashMap<String, Binding>>>,
    observers: Arc<Mutex<HashMap<String, ObserverEntry>>>,
}

impl SubscriberSet {
    fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let filter = BindingFilter::parse(&config.config)?;
        self.lock().insert(
            config.id,
            Binding {
                function_id: config.function_id,
                namespace: config.namespace,
                filter,
                metadata: config.metadata,
            },
        );
        Ok(())
    }

    fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    fn add_unfiltered(&self, config: TriggerConfig) -> Binding {
        let binding = Binding {
            function_id: config.function_id,
            namespace: config.namespace,
            filter: BindingFilter::default(),
            metadata: config.metadata,
        };
        self.lock().insert(config.id, binding.clone());
        binding
    }

    fn snapshot(&self) -> Vec<Binding> {
        self.lock().values().cloned().collect()
    }

    /// Parse an observer's args (the trigger config) and file it. Malformed
    /// args reject the observer, as they reject a binding.
    fn add_observer(&self, observer: Arc<dyn EventObserver>, args: &Value) -> Result<(), String> {
        let filter = BindingFilter::parse(args)?;
        self.file_observer(observer, filter);
        Ok(())
    }

    fn file_observer(&self, observer: Arc<dyn EventObserver>, filter: BindingFilter) {
        let id = observer.id().to_string();
        self.observers_lock()
            .insert(id, ObserverEntry { observer, filter });
    }

    fn remove_observer(&self, id: &str) {
        self.observers_lock().remove(id);
    }

    fn observer_snapshot(&self) -> Vec<ObserverEntry> {
        self.observers_lock().values().cloned().collect()
    }

    fn observers_lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, ObserverEntry>> {
        self.observers.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Binding>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

struct ReadyTriggerHandler {
    set: SubscriberSet,
    ready: Arc<AtomicBool>,
    iii: Arc<IIIClient>,
}

#[async_trait]
impl TriggerHandler for ReadyTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let raw = if config.config.is_null() {
            serde_json::json!({})
        } else {
            config.config.clone()
        };
        serde_json::from_value::<ReadyBindingConfig>(raw)
            .map_err(|error| Error::Handler(format!("invalid ready-event config: {error}")))?;
        let binding = self.set.add_unfiltered(config);
        if self.ready.load(Ordering::Acquire) {
            deliver_ready(&self.iii, &binding).await;
        }
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.remove(&config.id);
        Ok(())
    }
}

struct TurnEventTriggerHandler {
    type_id: &'static str,
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for TurnEventTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        // Trigger delivery never creates an agent — refuse the binding loudly
        // at registration instead of letting it no-op (or worse) at fire
        // time. This is the engine-direct path; agent registrations are
        // already rejected wholesale by the subscribe interceptor.
        if function_id == crate::functions::SPAWN_ID {
            return Err(Error::Handler(
                "harness::spawn is not a trigger target: trigger delivery never creates an \
                 agent. Spawn children directly and watch what they write."
                    .to_string(),
            ));
        }
        self.set.add(config).map_err(Error::Handler)?;
        tracing::info!(trigger_type = self.type_id, %id, %function_id, "turn-event subscription registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.remove(&config.id);
        Ok(())
    }
}

/// The ready config an observer's args must parse as (null means none).
fn parse_ready_config(raw: &Value) -> Result<ReadyBindingConfig, String> {
    let raw = if raw.is_null() {
        serde_json::json!({})
    } else {
        raw.clone()
    };
    serde_json::from_value::<ReadyBindingConfig>(raw)
        .map_err(|error| format!("invalid ready-event config: {error}"))
}

/// Files each observer of `harness::on-ready` in the ready subscriber set;
/// an observer added after boot receives the ready event at once, as a late
/// binding does.
struct ReadyObserveHandler {
    set: SubscriberSet,
    ready: Arc<AtomicBool>,
}

impl ReadyObserveHandler {
    fn add(&self, observer: Arc<dyn EventObserver>, args: &Value) -> Result<(), String> {
        parse_ready_config(args)?;
        self.set
            .file_observer(observer.clone(), BindingFilter::default());
        if self.ready.load(Ordering::Acquire) {
            observer.deliver(ready_payload());
        }
        Ok(())
    }
}

#[async_trait]
impl ObserveHandler for ReadyObserveHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        self.add(Arc::new(obs.clone()), obs.args_value())
            .map_err(|e| ObserveRejection::new(codes::INVALID_ARGS, e))?;
        tracing::info!(observer = %obs.id(), target = %obs.target(), function_id = %obs.function_id(), "observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.set.remove_observer(obs.id());
    }
}

/// Files each observer of a turn-event `harness::on-*` function in the
/// subscriber set its trigger type uses, under the same rules as a binding.
struct TurnEventObserveHandler {
    type_id: &'static str,
    set: SubscriberSet,
}

impl TurnEventObserveHandler {
    fn add(
        &self,
        observer: Arc<dyn EventObserver>,
        target: &str,
        args: &Value,
    ) -> Result<(), String> {
        // Same refusal as a binding: event delivery never creates an agent.
        if target == crate::functions::SPAWN_ID {
            return Err(
                "harness::spawn is not an observer target: event delivery never creates \
                 an agent. Spawn children directly and watch what they write."
                    .to_string(),
            );
        }
        self.set.add_observer(observer, args)
    }
}

#[async_trait]
impl ObserveHandler for TurnEventObserveHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        self.add(Arc::new(obs.clone()), obs.target(), obs.args_value())
            .map_err(|e| ObserveRejection::new(codes::INVALID_ARGS, e))?;
        tracing::info!(trigger_type = self.type_id, observer = %obs.id(), target = %obs.target(), "turn-event observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.set.remove_observer(obs.id());
    }
}

/// The harness's emitted turn-event subscriber sets + the engine handle for
/// fan-out. Cloned into [`crate::deps::Deps`].
#[derive(Clone)]
pub struct TurnEvents {
    iii: Arc<IIIClient>,
    ready: SubscriberSet,
    ready_emitted: Arc<AtomicBool>,
    started: SubscriberSet,
    completed: SubscriberSet,
    queued: SubscriberSet,
    triggers_changed: SubscriberSet,
    compaction: SubscriberSet,
}

impl TurnEvents {
    /// Register the trigger types and return the emitter. Must run before
    /// function registration so the handlers capture the subscriber sets.
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        let ready = SubscriberSet::default();
        let ready_emitted = Arc::new(AtomicBool::new(false));
        let started = SubscriberSet::default();
        let completed = SubscriberSet::default();
        let queued = SubscriberSet::default();
        let triggers_changed = SubscriberSet::default();
        let compaction = SubscriberSet::default();

        // Kept for consumers that bind the trigger type, on 0.24 and 0.25
        // engines alike.
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                READY,
                "The harness completed boot and can accept turns.",
                ReadyTriggerHandler {
                    set: ready.clone(),
                    ready: ready_emitted.clone(),
                    iii: iii.clone(),
                },
            )
            .trigger_request_format::<ReadyBindingConfig>(),
        );
        // Kept for consumers that bind the trigger type, on 0.24 and 0.25
        // engines alike.
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                TURN_STARTED,
                "A harness turn began executing (first loop step).",
                TurnEventTriggerHandler {
                    type_id: TURN_STARTED,
                    set: started.clone(),
                },
            )
            .trigger_request_format::<TurnEventBindingConfig>(),
        );
        // Kept for consumers that bind the trigger type, on 0.24 and 0.25
        // engines alike.
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                TURN_COMPLETED,
                "A harness turn reached a terminal status (completed/cancelled/failed).",
                TurnEventTriggerHandler {
                    type_id: TURN_COMPLETED,
                    set: completed.clone(),
                },
            )
            .trigger_request_format::<TurnEventBindingConfig>(),
        );
        // Kept for consumers that bind the trigger type, on 0.24 and 0.25
        // engines alike.
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                MESSAGE_QUEUED,
                "A message parked in a session's server-side queue while its turn streams.",
                TurnEventTriggerHandler {
                    type_id: MESSAGE_QUEUED,
                    set: queued.clone(),
                },
            )
            .trigger_request_format::<TurnEventBindingConfig>(),
        );
        // Kept for consumers that bind the trigger type, on 0.24 and 0.25
        // engines alike.
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                TRIGGERS_CHANGED,
                "A session's trigger-binding set or fire count changed — refetch harness::triggers::list.",
                TurnEventTriggerHandler {
                    type_id: TRIGGERS_CHANGED,
                    set: triggers_changed.clone(),
                },
            )
            .trigger_request_format::<TurnEventBindingConfig>(),
        );
        // Kept for consumers that bind the trigger type, on 0.24 and 0.25
        // engines alike.
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                COMPACTION,
                "An explicit compaction of a session started (`phase: started`) or finished \
                 (`phase: completed`, with `status` ok/busy/empty/failed and the user-facing `message`).",
                TurnEventTriggerHandler {
                    type_id: COMPACTION,
                    set: compaction.clone(),
                },
            )
            .trigger_request_format::<TurnEventBindingConfig>(),
        );
        tracing::info!(
            "registered harness::ready / harness::turn-started / harness::turn-completed / harness::message-queued / harness::triggers-changed / harness::compaction trigger types"
        );

        observe::register_observable::<ReadyBindingConfig, ReadyEvent, _>(
            iii,
            ON_READY,
            ON_READY_DESC,
            ReadyObserveHandler {
                set: ready.clone(),
                ready: ready_emitted.clone(),
            },
        );
        let turn_observer = |type_id: &'static str, set: &SubscriberSet| TurnEventObserveHandler {
            type_id,
            set: set.clone(),
        };
        observe::register_observable::<TurnEventBindingConfig, TurnStartedEvent, _>(
            iii,
            ON_TURN_STARTED,
            ON_TURN_STARTED_DESC,
            turn_observer(TURN_STARTED, &started),
        );
        observe::register_observable::<TurnEventBindingConfig, TurnCompletedEvent, _>(
            iii,
            ON_TURN_COMPLETED,
            ON_TURN_COMPLETED_DESC,
            turn_observer(TURN_COMPLETED, &completed),
        );
        observe::register_observable::<TurnEventBindingConfig, MessageQueuedEvent, _>(
            iii,
            ON_MESSAGE_QUEUED,
            ON_MESSAGE_QUEUED_DESC,
            turn_observer(MESSAGE_QUEUED, &queued),
        );
        observe::register_observable::<TurnEventBindingConfig, TriggersChangedEvent, _>(
            iii,
            ON_TRIGGERS_CHANGED,
            ON_TRIGGERS_CHANGED_DESC,
            turn_observer(TRIGGERS_CHANGED, &triggers_changed),
        );
        observe::register_observable::<TurnEventBindingConfig, CompactionEvent, _>(
            iii,
            ON_COMPACTION,
            ON_COMPACTION_DESC,
            turn_observer(COMPACTION, &compaction),
        );

        Self {
            iii: iii.clone(),
            ready,
            ready_emitted,
            started,
            completed,
            queued,
            triggers_changed,
            compaction,
        }
    }

    /// Announce readiness after all turn dependencies and lifecycle bindings
    /// have been installed. Subscribers that bind later receive the current
    /// ready state from [`ReadyTriggerHandler`].
    pub async fn emit_ready(&self) {
        self.ready_emitted.store(true, Ordering::Release);
        for binding in self.ready.snapshot() {
            deliver_ready(&self.iii, &binding).await;
        }
        for entry in self.ready.observer_snapshot() {
            entry.observer.deliver(ready_payload());
        }
    }

    /// A message parked in the session's `harness_queue` (send's queue path).
    /// The payload is a pointer, not the message — consumers refetch
    /// `harness::status` → `queued` (idempotent under at-least-once delivery).
    pub async fn emit_queued(&self, session_id: &str, entry_id: &str, queued_at: i64) {
        tracing::info!(session_id, entry_id, "message queued");
        let payload = serde_json::json!({
            "session_id": session_id,
            "entry_id": entry_id,
            "queued_at": queued_at,
            "timestamp": now_ms(),
        });
        self.fan_out(
            &self.queued,
            MESSAGE_QUEUED,
            session_id,
            None,
            None,
            payload,
        )
        .await;
    }

    /// Doorbell only — a session's binding set or fires count changed;
    /// consumers refetch `harness::triggers::list`. No per-event logging:
    /// a single fire rings twice (claim + retirement).
    pub async fn emit_triggers_changed(&self, session_id: &str) {
        let payload = serde_json::json!({
            "session_id": session_id,
            "timestamp": now_ms(),
        });
        self.fan_out(
            &self.triggers_changed,
            TRIGGERS_CHANGED,
            session_id,
            None,
            None,
            payload,
        )
        .await;
    }

    /// An explicit compaction's progress (`crate::compaction::event_payload`).
    /// Every client also gets it durably: the start notice and the outcome
    /// row land in the transcript; this is the live signal for consumers that
    /// track the operation itself.
    pub async fn emit_compaction(&self, session_id: &str, payload: Value) {
        self.fan_out(
            &self.compaction,
            COMPACTION,
            session_id,
            None,
            None,
            payload,
        )
        .await;
    }

    /// `message_preview` is the first characters of the user message that
    /// started the turn (a sub-agent's task text for a child), so a consumer
    /// can name the turn without reading the transcript; `depth` is 0 for a
    /// top-level turn.
    pub async fn emit_started(
        &self,
        session_id: &str,
        turn_id: &str,
        parent: Option<&ParentLink>,
        display_parent: Option<&str>,
        message_preview: Option<&str>,
        depth: u32,
    ) {
        tracing::info!(session_id, turn_id, "turn started");
        let mut payload = serde_json::json!({
            "session_id": session_id,
            "turn_id": turn_id,
            "timestamp": now_ms(),
            "depth": depth,
        });
        if let Some(preview) = message_preview.filter(|p| !p.trim().is_empty()) {
            payload["message_preview"] = Value::String(preview.to_string());
        }
        if let Some(p) = parent {
            payload["parent"] = serde_json::to_value(p).unwrap_or(Value::Null);
        }
        if let Some(dp) = display_parent {
            payload["parent_session_id"] = Value::String(dp.to_string());
        }
        self.fan_out(
            &self.started,
            TURN_STARTED,
            session_id,
            parent,
            display_parent,
            payload,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn emit_completed(
        &self,
        session_id: &str,
        turn_id: &str,
        status: &str,
        result: Option<&Value>,
        result_error: Option<&str>,
        reason: Option<&str>,
        parent: Option<&ParentLink>,
        display_parent: Option<&str>,
        terminal: bool,
        context: Option<&crate::context_snapshot::ContextSnapshotV1>,
    ) {
        tracing::info!(
            session_id,
            turn_id,
            status,
            result_error,
            terminal,
            "turn completed"
        );
        // `terminal: false` marks a turn whose session still owns an armed
        // wake (a one-shot notify) — the run continues and a later turn in the
        // same session carries the real outcome. Consumers should treat
        // non-terminal completions as progress and finalize only on
        // `terminal: true`.
        let mut payload = serde_json::json!({
            "session_id": session_id,
            "turn_id": turn_id,
            "status": status,
            "terminal": terminal,
            "timestamp": now_ms(),
        });
        if let Some(r) = result {
            payload["result"] = r.clone();
        }
        if let Some(e) = result_error {
            payload["result_error"] = Value::String(e.to_string());
        }
        if let Some(r) = reason {
            payload["reason"] = Value::String(r.to_string());
        }
        if let Some(p) = parent {
            payload["parent"] = serde_json::to_value(p).unwrap_or(Value::Null);
        }
        if let Some(dp) = display_parent {
            payload["parent_session_id"] = Value::String(dp.to_string());
        }
        // The latest generation's context accounting (categories, budget,
        // usage) so live consumers never re-walk the transcript for it.
        if let Some(snapshot) = context {
            payload["context"] = serde_json::to_value(snapshot).unwrap_or(Value::Null);
        }
        self.fan_out(
            &self.completed,
            TURN_COMPLETED,
            session_id,
            parent,
            display_parent,
            payload,
        )
        .await;
    }

    async fn fan_out(
        &self,
        set: &SubscriberSet,
        trigger_type: &str,
        session_id: &str,
        parent: Option<&ParentLink>,
        display_parent: Option<&str>,
        payload: Value,
    ) {
        for binding in set.snapshot() {
            if !binding.filter.matches(session_id, parent, display_parent) {
                continue;
            }
            // The registration metadata is forwarded as stored; there is no
            // fire-time stamping.
            let request = TriggerRequest {
                function_id: binding.function_id.clone(),
                payload: payload.clone(),
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            };
            let request: iii_sdk::protocol::TriggerRequestWithMetadata =
                match binding.metadata.clone() {
                    Some(metadata) => request.metadata(metadata),
                    None => request.into(),
                };
            let namespace = binding.namespace.as_deref().unwrap_or("default");
            let res = self.iii.trigger(request.namespace(namespace)).await;
            if let Err(e) = res {
                tracing::warn!(trigger_type, function_id = %binding.function_id, error = %e, "turn-event fan-out failed");
            }
        }
        for entry in set.observer_snapshot() {
            if entry.filter.matches(session_id, parent, display_parent) {
                entry.observer.deliver(payload.clone());
            }
        }
    }
}

async fn deliver_ready(iii: &IIIClient, binding: &Binding) {
    let request = TriggerRequest {
        function_id: binding.function_id.clone(),
        payload: ready_payload(),
        action: Some(TriggerAction::Void),
        timeout_ms: None,
    };
    let request: iii_sdk::protocol::TriggerRequestWithMetadata = match &binding.metadata {
        Some(metadata) => request.metadata(metadata.clone()),
        None => request.into(),
    };
    let namespace = binding.namespace.as_deref().unwrap_or("default");
    let result = iii.trigger(request.namespace(namespace)).await;
    if let Err(error) = result {
        tracing::warn!(
            trigger_type = READY,
            function_id = %binding.function_id,
            %error,
            "ready-event fan-out failed"
        );
    }
}

/// The `harness::ready` event, for bindings and observers alike.
fn ready_payload() -> Value {
    serde_json::json!({
        "status": "ready",
        "timestamp": now_ms(),
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filter_matches_session_and_parent() {
        let f = BindingFilter {
            session_id: Some("s_1".into()),
            parent_session_id: None,
        };
        assert!(f.matches("s_1", None, None));
        assert!(!f.matches("s_2", None, None));

        let pf = BindingFilter {
            session_id: None,
            parent_session_id: Some("p_1".into()),
        };
        let parent = ParentLink {
            session_id: "p_1".into(),
            turn_id: "t".into(),
            function_call_id: "fc".into(),
        };
        assert!(pf.matches("child", Some(&parent), None));
        assert!(!pf.matches("child", None, None));
    }

    #[test]
    fn filter_matches_display_parent_for_trigger_fired_children() {
        let pf = BindingFilter {
            session_id: None,
            parent_session_id: Some("root_1".into()),
        };
        // Trigger-spawned child: no ParentLink, display parent only.
        assert!(pf.matches("child", None, Some("root_1")));
        assert!(!pf.matches("child", None, Some("other_root")));
    }

    #[test]
    fn parse_rejects_unknown_fields() {
        assert!(BindingFilter::parse(&json!({ "sesion_id": "x" })).is_err());
        assert_eq!(
            BindingFilter::parse(&Value::Null).unwrap(),
            BindingFilter::default()
        );
    }

    #[test]
    fn subscriber_preserves_target_namespace() {
        let set = SubscriberSet::default();
        set.add(TriggerConfig {
            id: "sub-1".into(),
            function_id: "ui::refresh".into(),
            config: json!({}),
            metadata: None,
            namespace: Some("project-a".into()),
        })
        .unwrap();
        assert_eq!(set.snapshot()[0].namespace.as_deref(), Some("project-a"));
    }
}

#[cfg(test)]
mod observable_tests {
    use super::*;
    use serde_json::json;

    /// An observer of a `harness::on-*` function that records its events.
    struct Recorder {
        id: String,
        seen: Mutex<Vec<Value>>,
    }

    impl EventObserver for Recorder {
        fn id(&self) -> &str {
            &self.id
        }

        fn deliver(&self, event: Value) {
            self.seen.lock().unwrap().push(event);
        }
    }

    fn recorder(id: &str) -> Arc<Recorder> {
        Arc::new(Recorder {
            id: id.to_string(),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn seen(r: &Recorder) -> Vec<Value> {
        r.seen.lock().unwrap().clone()
    }

    /// An emitter over a client that never connects: binding fires are void
    /// triggers that go nowhere, observers record.
    fn emitter() -> TurnEvents {
        TurnEvents {
            iii: Arc::new(IIIClient::new("ws://127.0.0.1:9")),
            ready: SubscriberSet::default(),
            ready_emitted: Arc::new(AtomicBool::new(false)),
            started: SubscriberSet::default(),
            completed: SubscriberSet::default(),
            queued: SubscriberSet::default(),
            triggers_changed: SubscriberSet::default(),
            compaction: SubscriberSet::default(),
        }
    }

    fn binding(id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: format!("probe::{id}"),
            config,
            metadata: None,
            namespace: None,
        }
    }

    #[test]
    fn each_on_function_refuses_direct_invocation() {
        let pairs = [
            (READY, ON_READY),
            (TURN_STARTED, ON_TURN_STARTED),
            (TURN_COMPLETED, ON_TURN_COMPLETED),
            (MESSAGE_QUEUED, ON_MESSAGE_QUEUED),
            (TRIGGERS_CHANGED, ON_TRIGGERS_CHANGED),
            (COMPACTION, ON_COMPACTION),
        ];
        for (trigger_type, function_id) in pairs {
            assert_eq!(
                function_id,
                trigger_type.replacen("harness::", "harness::on-", 1)
            );
            let err = crate::observe::refuse_invocation(function_id);
            assert!(
                matches!(&err, Error::Remote { code, message, .. }
                    if code == codes::FORBIDDEN && message.contains(function_id)),
                "{err:?}"
            );
        }
    }

    /// Every event an observer receives parses as the request type its
    /// on-* function publishes, so the published schema documents it.
    #[tokio::test]
    async fn every_delivered_event_parses_as_its_published_request_type() {
        let events = emitter();
        let r = recorder("o1");
        for set in [
            &events.ready,
            &events.started,
            &events.completed,
            &events.queued,
            &events.triggers_changed,
            &events.compaction,
        ] {
            set.file_observer(r.clone(), BindingFilter::default());
        }
        let parent = ParentLink {
            session_id: "p1".into(),
            turn_id: "pt".into(),
            function_call_id: "fc".into(),
        };
        let outcome = crate::compaction::Outcome {
            session_id: "s1".into(),
            compaction_id: "c1".into(),
            status: crate::compaction::Status::Ok,
            message: "done".into(),
            tokens_before: Some(10),
            tokens_after: Some(2),
            summary: None,
            entry_ids: vec!["e1".into()],
        };
        events.emit_ready().await;
        events
            .emit_started("s1", "t1", Some(&parent), Some("p1"), Some("hi"), 1)
            .await;
        events
            .emit_completed(
                "s1",
                "t1",
                "completed",
                Some(&json!({ "answer": 42 })),
                None,
                None,
                Some(&parent),
                Some("p1"),
                true,
                None,
            )
            .await;
        events.emit_queued("s1", "q1", 5).await;
        events.emit_triggers_changed("s1").await;
        events
            .emit_compaction("s1", crate::compaction::event_payload("started", &outcome))
            .await;
        events
            .emit_compaction(
                "s1",
                crate::compaction::event_payload("completed", &outcome),
            )
            .await;
        let seen = seen(&r);
        assert_eq!(seen.len(), 7, "{seen:?}");
        serde_json::from_value::<ReadyEvent>(seen[0].clone()).unwrap();
        let started = serde_json::from_value::<TurnStartedEvent>(seen[1].clone()).unwrap();
        assert_eq!(started.parent, Some(parent.clone()));
        let completed = serde_json::from_value::<TurnCompletedEvent>(seen[2].clone()).unwrap();
        assert!(completed.terminal);
        serde_json::from_value::<MessageQueuedEvent>(seen[3].clone()).unwrap();
        serde_json::from_value::<TriggersChangedEvent>(seen[4].clone()).unwrap();
        serde_json::from_value::<CompactionEvent>(seen[5].clone()).unwrap();
        let finished = serde_json::from_value::<CompactionEvent>(seen[6].clone()).unwrap();
        assert_eq!(finished.status, Some(crate::compaction::Status::Ok));
    }

    #[tokio::test]
    async fn a_turn_completed_observer_receives_the_event_a_binding_receives() {
        let events = emitter();
        events
            .completed
            .add(binding("b1", json!({ "session_id": "s1" })))
            .unwrap();
        let mine = recorder("o-mine");
        let children = recorder("o-children");
        let other = recorder("o-other");
        let started_only = recorder("o-started");
        events
            .completed
            .add_observer(mine.clone(), &json!({ "session_id": "s1" }))
            .unwrap();
        events
            .completed
            .add_observer(children.clone(), &json!({ "parent_session_id": "p1" }))
            .unwrap();
        events
            .completed
            .add_observer(other.clone(), &json!({ "session_id": "s2" }))
            .unwrap();
        events
            .started
            .add_observer(started_only.clone(), &Value::Null)
            .unwrap();

        events
            .emit_completed(
                "s1",
                "t1",
                "completed",
                Some(&json!({ "text": "done" })),
                None,
                None,
                None,
                Some("p1"),
                true,
                None,
            )
            .await;

        // The binding matched the same filter the observer did.
        let bound: Vec<_> = events
            .completed
            .snapshot()
            .into_iter()
            .filter(|b| b.filter.matches("s1", None, Some("p1")))
            .map(|b| b.function_id)
            .collect();
        assert_eq!(bound, ["probe::b1"]);
        let got = seen(&mine);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["session_id"], json!("s1"));
        assert_eq!(got[0]["turn_id"], json!("t1"));
        assert_eq!(got[0]["status"], json!("completed"));
        assert_eq!(got[0]["terminal"], json!(true));
        assert_eq!(got[0]["result"], json!({ "text": "done" }));
        assert_eq!(got[0]["parent_session_id"], json!("p1"));
        assert_eq!(
            seen(&children),
            got,
            "display parent matches like a binding"
        );
        assert!(seen(&other).is_empty());
        assert!(seen(&started_only).is_empty());

        events.completed.remove_observer("o-mine");
        events
            .emit_completed(
                "s1",
                "t2",
                "completed",
                None,
                None,
                None,
                None,
                None,
                true,
                None,
            )
            .await;
        assert_eq!(seen(&mine).len(), 1);
        assert_eq!(events.completed.snapshot().len(), 1, "bindings unchanged");
    }

    #[tokio::test]
    async fn the_other_turn_events_reach_their_observers() {
        let events = emitter();
        let started = recorder("started");
        let queued = recorder("queued");
        let changed = recorder("changed");
        let compaction = recorder("compaction");
        events
            .started
            .add_observer(started.clone(), &json!({}))
            .unwrap();
        events
            .queued
            .add_observer(queued.clone(), &json!({}))
            .unwrap();
        events
            .triggers_changed
            .add_observer(changed.clone(), &json!({}))
            .unwrap();
        events
            .compaction
            .add_observer(compaction.clone(), &json!({}))
            .unwrap();

        events
            .emit_started("s1", "t1", None, None, Some("hello"), 0)
            .await;
        events.emit_queued("s1", "e1", 5).await;
        events.emit_triggers_changed("s1").await;
        events
            .emit_compaction("s1", json!({ "phase": "started" }))
            .await;

        assert_eq!(seen(&started)[0]["message_preview"], json!("hello"));
        assert_eq!(seen(&queued)[0]["entry_id"], json!("e1"));
        assert_eq!(seen(&changed)[0]["session_id"], json!("s1"));
        assert_eq!(seen(&compaction), vec![json!({ "phase": "started" })]);
    }

    #[tokio::test]
    async fn on_ready_observers_get_the_ready_event_like_bindings() {
        let events = emitter();
        let handler = ReadyObserveHandler {
            set: events.ready.clone(),
            ready: events.ready_emitted.clone(),
        };
        let early = recorder("early");
        handler.add(early.clone(), &Value::Null).unwrap();
        assert!(seen(&early).is_empty(), "not ready yet");
        events.emit_ready().await;
        assert_eq!(seen(&early).len(), 1);
        assert_eq!(seen(&early)[0]["status"], json!("ready"));
        // Added after boot: delivered at once, as a late binding is.
        let late = recorder("late");
        handler.add(late.clone(), &json!({})).unwrap();
        assert_eq!(seen(&late)[0]["status"], json!("ready"));
        // Args the ready trigger type refuses reject the observer.
        let err = handler
            .add(recorder("bad"), &json!({ "session_id": "s1" }))
            .unwrap_err();
        assert!(err.contains("invalid ready-event config"), "{err}");
    }

    #[test]
    fn turn_event_observers_follow_the_binding_rules() {
        let set = SubscriberSet::default();
        let handler = TurnEventObserveHandler {
            type_id: TURN_COMPLETED,
            set: set.clone(),
        };
        let err = handler
            .add(recorder("spawn"), crate::functions::SPAWN_ID, &json!({}))
            .unwrap_err();
        assert!(err.contains("harness::spawn"), "{err}");
        let err = handler
            .add(
                recorder("typo"),
                "ui::refresh",
                &json!({ "sesion_id": "s1" }),
            )
            .unwrap_err();
        assert!(err.contains("invalid turn-event config"), "{err}");
        handler
            .add(
                recorder("ok"),
                "ui::refresh",
                &json!({ "session_id": "s1" }),
            )
            .unwrap();
        assert_eq!(set.observer_snapshot().len(), 1);
        assert!(set.snapshot().is_empty(), "observers are not bindings");
    }
}
