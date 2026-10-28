//! The session trigger types this worker emits, the observable
//! `computer::on-*` function next to each one, and the fan-out behind them.
//! Old consumers bind a trigger type with the standard two-step pattern; the
//! engine routes each registration to our [`TriggerHandler`]. New consumers
//! observe the function (iii 0.25) with the same config as args; its
//! [`ObserveHandler`] files observers in the same subscriber set. Delivery is
//! fire-and-forget and at-least-once for bindings (`TriggerAction::Void`),
//! fire-and-forget `obs.call` for observers; the optional `session_id` filter
//! is evaluated here by the emitting worker, and malformed configs or args are
//! rejected at registration time. Events are routed in-process: invoking an
//! on-* function is refused with FORBIDDEN, so nobody can forge one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
    RegisterTriggerType, TriggerAction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::driver::Screen;

pub const SESSION_STARTED: &str = "computer::session-started";
pub const SESSION_STOPPED: &str = "computer::session-stopped";
pub const ACTIVITY: &str = "computer::activity";
pub const ACTIVITY_DESC: &str =
    "A caller drove a desktop session (computer::act or computer::run); the console's live \
     preview shows that session.";

pub const ON_SESSION_STARTED: &str = "computer::on-session-started";
pub const ON_SESSION_STOPPED: &str = "computer::on-session-stopped";
pub const ON_ACTIVITY: &str = "computer::on-activity";
pub const ON_SESSION_STARTED_DESC: &str =
    "A desktop session connected and is ready to drive. Observe it with args { session_id? }; \
     each observer receives { session_id, endpoint, os, screen, timestamp }. Only the computer \
     worker emits it: invoking it directly is refused.";
pub const ON_SESSION_STOPPED_DESC: &str =
    "A desktop session ended. Observe it with args { session_id? }; each observer receives \
     { session_id, reason: stopped | idle, timestamp }. Only the computer worker emits it: \
     invoking it directly is refused.";
pub const ON_ACTIVITY_DESC: &str =
    "A caller drove a desktop session (computer::act or computer::run). Observe it with args \
     { session_id? }; each observer receives { session_id, function, timestamp }. Only the \
     computer worker emits it: invoking it directly is refused.";

/// Codes the `computer::on-*` functions refuse with.
pub mod codes {
    /// Invoking an on-* function directly.
    pub const FORBIDDEN: &str = "FORBIDDEN";
    /// An observer's args the trigger type would refuse as a config.
    pub const INVALID_ARGS: &str = "INVALID_ARGS";
    /// `computer::on-frame-changed`: the binding and observer cap is reached.
    pub const LIMIT_REACHED: &str = "LIMIT_REACHED";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    SessionStarted,
    SessionStopped,
    Activity,
}

impl EventKind {
    pub fn trigger_type(&self) -> &'static str {
        match self {
            EventKind::SessionStarted => SESSION_STARTED,
            EventKind::SessionStopped => SESSION_STOPPED,
            EventKind::Activity => ACTIVITY,
        }
    }

    /// The observable function next to the trigger type.
    pub fn observable_function(&self) -> &'static str {
        match self {
            EventKind::SessionStarted => ON_SESSION_STARTED,
            EventKind::SessionStopped => ON_SESSION_STOPPED,
            EventKind::Activity => ON_ACTIVITY,
        }
    }

    pub fn all() -> [EventKind; 3] {
        [
            EventKind::SessionStarted,
            EventKind::SessionStopped,
            EventKind::Activity,
        ]
    }
}

/// Config accepted by every `computer::*` trigger binding. The only filter is
/// an optional session-id equality match; unknown fields fail at registration
/// so a misspelled filter key fails loudly instead of silently receiving
/// nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingConfig {
    /// Only deliver events for this computer session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

pub fn binding_matches(filter: &BindingConfig, session_id: &str) -> bool {
    match &filter.session_id {
        Some(want) => want == session_id,
        None => true,
    }
}

// ---------------------------------------------------------------------------
// Event payloads (what subscribers receive)
// ---------------------------------------------------------------------------

/// `computer::session-started` — a desktop session is connected and ready.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStartedEvent {
    pub session_id: String,
    /// The endpoint the session is driving (`native` for the local machine).
    pub endpoint: String,
    /// Informational guest OS label (`linux`, `macos`, ...).
    pub os: String,
    pub screen: Screen,
    pub timestamp: i64,
}

/// `computer::activity` — a caller drove a session: fired when a
/// `computer::act` call reaches it, or a `computer::run` passed its checks and
/// holds it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ActivityEvent {
    pub session_id: String,
    /// The function called: `computer::act` or `computer::run`.
    pub function: String,
    pub timestamp: i64,
}

/// `computer::session-stopped` — a session ended; see `reason`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStoppedEvent {
    pub session_id: String,
    /// One of `stopped`, `idle`.
    pub reason: String,
    pub timestamp: i64,
}

// ---------------------------------------------------------------------------
// Subscriber registry + trigger type registration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Binding {
    pub id: String,
    pub function_id: String,
    pub filter: BindingConfig,
}

/// One observer of a `computer::on-*` function with its parsed args.
#[derive(Clone)]
pub struct ObserverBinding {
    pub observer: Arc<dyn EventObserver>,
    pub filter: BindingConfig,
}

/// Thread-safe subscriber registry for one trigger type: its bindings and the
/// observers of its `computer::on-*` function, kept apart by id.
#[derive(Clone)]
pub struct SubscriberSet {
    kind: EventKind,
    inner: Arc<Mutex<HashMap<String, Binding>>>,
    observers: Arc<Mutex<HashMap<String, ObserverBinding>>>,
}

impl SubscriberSet {
    pub fn new(kind: EventKind) -> Self {
        Self {
            kind,
            inner: Arc::new(Mutex::new(HashMap::new())),
            observers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// A binding's config or an observer's args; null means no filter.
    fn parse_filter(&self, raw: Value) -> Result<BindingConfig, String> {
        let raw = if raw.is_null() {
            Value::Object(serde_json::Map::new())
        } else {
            raw
        };
        serde_json::from_value(raw)
            .map_err(|e| format!("invalid {} config: {e}", self.kind.trigger_type()))
    }

    /// Parse + insert a binding. Rejects malformed configs at registration.
    pub fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let filter = self.parse_filter(config.config.clone())?;
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

    /// Parse an observer's args (the trigger config) and file it. Malformed
    /// args reject the observer.
    pub fn add_observer(
        &self,
        observer: Arc<dyn EventObserver>,
        args: Value,
    ) -> Result<(), String> {
        let filter = self.parse_filter(args)?;
        let id = observer.id().to_string();
        self.observers_lock()
            .insert(id, ObserverBinding { observer, filter });
        Ok(())
    }

    pub fn remove_observer(&self, id: &str) {
        self.observers_lock().remove(id);
    }

    /// Snapshot so the mutex is never held across awaits.
    pub fn snapshot(&self) -> Vec<Binding> {
        self.lock().values().cloned().collect()
    }

    /// Snapshot of the observers, same rule.
    pub fn observer_snapshot(&self) -> Vec<ObserverBinding> {
        self.observers_lock().values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Binding>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn observers_lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, ObserverBinding>> {
        self.observers.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The two subscriber sets, one per trigger type.
#[derive(Clone)]
pub struct TriggerSets {
    inner: HashMap<&'static str, SubscriberSet>,
}

impl TriggerSets {
    pub fn new() -> Self {
        let mut inner = HashMap::new();
        for kind in EventKind::all() {
            inner.insert(kind.trigger_type(), SubscriberSet::new(kind));
        }
        Self { inner }
    }

    pub fn for_kind(&self, kind: EventKind) -> &SubscriberSet {
        self.inner
            .get(kind.trigger_type())
            .expect("all kinds are populated at construction")
    }
}

impl Default for TriggerSets {
    fn default() -> Self {
        Self::new()
    }
}

struct ComputerTriggerHandler {
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for ComputerTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        self.set.add(config).map_err(Error::Handler)?;
        tracing::info!(id = %id, function_id = %function_id, "trigger subscription registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(id = %config.id, "trigger subscription unregistered");
        self.set.remove(&config.id);
        Ok(())
    }
}

/// One observer of a `computer::on-*` function, as the emitter sees it.
/// Production wraps the SDK's [`Observer`]; tests record.
#[async_trait]
pub trait EventObserver: Send + Sync {
    fn id(&self) -> &str;
    /// Hand one event to the observer. Fire-and-forget like a binding's
    /// delivery: never blocks the caller that produced the event.
    async fn deliver(&self, payload: Value);
}

#[async_trait]
impl EventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    async fn deliver(&self, payload: Value) {
        let obs = self.clone();
        tokio::spawn(async move {
            match obs.call::<Value>(payload).await {
                Ok(_) => {}
                // The observer's own filters declined this event.
                Err(Error::Remote { code, .. }) if code == "FILTERS_NOT_MATCHED" => {}
                Err(e) => {
                    tracing::warn!(observer = %obs.id(), function_id = %obs.function_id(), error = %e, "observer delivery failed");
                }
            }
        });
    }
}

/// Files each observer of a `computer::on-*` function in the subscriber set
/// its trigger type uses, and takes it out again when the engine removes it.
struct ComputerObserveHandler {
    set: SubscriberSet,
}

#[async_trait]
impl ObserveHandler for ComputerObserveHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        self.set
            .add_observer(Arc::new(obs.clone()), obs.args_value().clone())
            .map_err(|e| ObserveRejection::new(codes::INVALID_ARGS, e))?;
        tracing::info!(observer = %obs.id(), target = %obs.target(), function_id = %obs.function_id(), "observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.set.remove_observer(obs.id());
        tracing::info!(observer = %obs.id(), function_id = %obs.function_id(), "observer removed");
    }
}

/// The error every `computer::on-*` handler returns: events come only from
/// this worker, in-process, never by invoking the function.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: codes::FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args }}, target)) instead of invoking \
             it; only the computer worker emits its events"
        ),
        stacktrace: None,
    }
}

/// The handler registered for an on-* function: refuses any payload, of any
/// shape, with FORBIDDEN (it takes a raw [`Value`], so no parse error comes
/// first).
pub fn observable_handler(
    function_id: &'static str,
) -> impl Fn(Value) -> Result<(), Error> + Send + Sync + 'static {
    move |_payload: Value| Err(refuse_invocation(function_id))
}

/// JSON Schema of the event an observer receives, documented as the
/// function's request format.
pub(crate) fn event_schema<E: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(E)).unwrap_or(Value::Null)
}

/// Registers `kind`'s observable function, owner-delivered, with args
/// [`BindingConfig`] (the trigger config) and hooks filing observers in the
/// trigger type's subscriber set. On an engine without observe it registers
/// as a plain function and its hooks never run.
fn register_observable<E>(iii: &IIIClient, kind: EventKind, description: &str, sets: &TriggerSets)
where
    E: JsonSchema,
{
    let function_id = kind.observable_function();
    iii.register_function(
        function_id,
        RegisterFunction::new(observable_handler(function_id))
            .description(description)
            .request_format(event_schema::<E>())
            .observe(Observe::owner().args_format::<BindingConfig>().handler(
                ComputerObserveHandler {
                    set: sets.for_kind(kind).clone(),
                },
            )),
    );
    tracing::info!(function_id, "registered observable function");
}

/// Register the custom trigger types with the engine, and the observable
/// `computer::on-*` function next to each one. Must run before
/// `functions::register_all` so handlers can capture the subscriber sets.
pub fn register_trigger_types(iii: &Arc<IIIClient>) -> TriggerSets {
    let sets = TriggerSets::new();
    let descriptions: [(EventKind, &str); 3] = [
        (
            EventKind::SessionStarted,
            "A desktop session connected and is ready to drive.",
        ),
        (
            EventKind::SessionStopped,
            "A desktop session ended (stopped or idle).",
        ),
        (EventKind::Activity, ACTIVITY_DESC),
    ];
    for (kind, description) in descriptions {
        // Kept for consumers that bind the trigger type, on 0.24 and 0.25
        // engines alike.
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                kind.trigger_type(),
                description,
                ComputerTriggerHandler {
                    set: sets.for_kind(kind).clone(),
                },
            )
            .trigger_request_format::<BindingConfig>(),
        );
        tracing::info!(
            trigger_type = kind.trigger_type(),
            "registered trigger type"
        );
    }
    register_observable::<SessionStartedEvent>(
        iii,
        EventKind::SessionStarted,
        ON_SESSION_STARTED_DESC,
        &sets,
    );
    register_observable::<SessionStoppedEvent>(
        iii,
        EventKind::SessionStopped,
        ON_SESSION_STOPPED_DESC,
        &sets,
    );
    register_observable::<ActivityEvent>(iii, EventKind::Activity, ON_ACTIVITY_DESC, &sets);
    sets
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// How a matched event reaches a subscriber. Production delivers over the bus;
/// tests record.
#[async_trait]
pub trait EventDeliverer: Send + Sync {
    async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value);
}

/// Fire-and-forget bus delivery so the caller that produced the event is never
/// blocked on subscriber latency. Failures are logged and swallowed.
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
    async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value) {
        let iii = self.iii.clone();
        let trigger_type = trigger_type.to_string();
        let function_id = function_id.to_string();
        tokio::spawn(async move {
            let res = iii
                .trigger(TriggerRequest {
                    function_id: function_id.clone(),
                    payload,
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                })
                .await;
            if let Err(e) = res {
                tracing::warn!(trigger_type, function_id, error = %e, "event fan-out failed");
            }
        });
    }
}

/// Evaluates each binding's session filter against each event and delivers the
/// payload to every match.
pub struct Emitter {
    sets: TriggerSets,
    deliverer: Arc<dyn EventDeliverer>,
}

impl Emitter {
    pub fn new(sets: TriggerSets, deliverer: Arc<dyn EventDeliverer>) -> Self {
        Self { sets, deliverer }
    }

    pub async fn emit<T: Serialize>(&self, kind: EventKind, session_id: &str, payload: &T) {
        let set = self.sets.for_kind(kind);
        let bindings: Vec<Binding> = set
            .snapshot()
            .into_iter()
            .filter(|b| binding_matches(&b.filter, session_id))
            .collect();
        let observers: Vec<ObserverBinding> = set
            .observer_snapshot()
            .into_iter()
            .filter(|o| binding_matches(&o.filter, session_id))
            .collect();
        if bindings.is_empty() && observers.is_empty() {
            return;
        }
        let payload = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "event payload failed to serialize");
                return;
            }
        };
        for binding in bindings {
            self.deliverer
                .deliver(kind.trigger_type(), &binding.function_id, payload.clone())
                .await;
        }
        for observer in observers {
            observer.observer.deliver(payload.clone()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn trigger_config(config: Value) -> TriggerConfig {
        TriggerConfig {
            id: "t1".to_string(),
            function_id: "console::on-event".to_string(),
            config,
            metadata: None,
            namespace: None,
        }
    }

    #[test]
    fn empty_filter_matches_everything() {
        assert!(binding_matches(&BindingConfig::default(), "c1"));
    }

    #[test]
    fn session_filter_is_equality() {
        let filter = BindingConfig {
            session_id: Some("c1".to_string()),
        };
        assert!(binding_matches(&filter, "c1"));
        assert!(!binding_matches(&filter, "c2"));
    }

    #[test]
    fn add_rejects_unknown_filter_keys() {
        let set = SubscriberSet::new(EventKind::SessionStarted);
        let err = set
            .add(trigger_config(json!({ "sesion_id": "c1" })))
            .unwrap_err();
        assert!(err.contains("computer::session-started"), "{err}");
    }

    #[test]
    fn add_accepts_null_config() {
        let set = SubscriberSet::new(EventKind::SessionStopped);
        set.add(trigger_config(Value::Null)).unwrap();
        assert_eq!(set.snapshot().len(), 1);
    }

    #[tokio::test]
    async fn emit_delivers_only_to_matching_bindings() {
        struct Recorder(Mutex<Vec<(String, String)>>);

        #[async_trait]
        impl EventDeliverer for Recorder {
            async fn deliver(&self, trigger_type: &str, function_id: &str, _payload: Value) {
                self.0
                    .lock()
                    .unwrap()
                    .push((trigger_type.to_string(), function_id.to_string()));
            }
        }

        let sets = TriggerSets::new();
        let set = sets.for_kind(EventKind::SessionStopped);
        set.add(TriggerConfig {
            id: "match".to_string(),
            function_id: "fn::match".to_string(),
            config: json!({ "session_id": "c1" }),
            metadata: None,
            namespace: None,
        })
        .unwrap();
        set.add(TriggerConfig {
            id: "other".to_string(),
            function_id: "fn::other".to_string(),
            config: json!({ "session_id": "c2" }),
            metadata: None,
            namespace: None,
        })
        .unwrap();

        let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
        let emitter = Emitter::new(sets, recorder.clone());
        emitter
            .emit(
                EventKind::SessionStopped,
                "c1",
                &SessionStoppedEvent {
                    session_id: "c1".to_string(),
                    reason: "stopped".to_string(),
                    timestamp: 0,
                },
            )
            .await;

        let seen = recorder.0.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![(SESSION_STOPPED.to_string(), "fn::match".to_string())]
        );
    }

    /// Records what bindings are handed (trigger type, function, payload).
    struct PayloadRecorder(Mutex<Vec<(String, Value)>>);

    #[async_trait]
    impl EventDeliverer for PayloadRecorder {
        async fn deliver(&self, _trigger_type: &str, function_id: &str, payload: Value) {
            self.0
                .lock()
                .unwrap()
                .push((function_id.to_string(), payload));
        }
    }

    /// An observer of a `computer::on-*` function that records its events.
    struct ObserverRecorder {
        id: String,
        seen: Mutex<Vec<Value>>,
    }

    #[async_trait]
    impl EventObserver for ObserverRecorder {
        fn id(&self) -> &str {
            &self.id
        }

        async fn deliver(&self, payload: Value) {
            self.seen.lock().unwrap().push(payload);
        }
    }

    fn observer(id: &str) -> Arc<ObserverRecorder> {
        Arc::new(ObserverRecorder {
            id: id.to_string(),
            seen: Mutex::new(Vec::new()),
        })
    }

    #[test]
    fn on_functions_refuse_direct_invocation() {
        // The very handler each on-* function is registered with: any
        // payload, junk included, is refused before any parsing.
        for kind in EventKind::all() {
            let function_id = kind.observable_function();
            let handler = observable_handler(function_id);
            for junk in [
                json!("junk"),
                json!(42),
                Value::Null,
                json!({ "session_id": "c1", "forged": true }),
                json!([1, 2, 3]),
            ] {
                let err = handler(junk).unwrap_err();
                assert!(
                    matches!(&err, Error::Remote { code, message, .. }
                        if code == codes::FORBIDDEN && message.contains(function_id)),
                    "{err:?}"
                );
            }
        }
        for kind in EventKind::all() {
            let function_id = kind.observable_function();
            let err = refuse_invocation(function_id);
            assert!(
                matches!(&err, Error::Remote { code, message, .. }
                    if code == codes::FORBIDDEN && message.contains(function_id)),
                "{err:?}"
            );
        }
        assert_eq!(
            EventKind::all().map(|kind| kind.observable_function()),
            [
                "computer::on-session-started",
                "computer::on-session-stopped",
                "computer::on-activity",
            ]
        );
        // Each sits next to its trigger type.
        for kind in EventKind::all() {
            assert_eq!(
                kind.observable_function(),
                kind.trigger_type()
                    .replacen("computer::", "computer::on-", 1)
            );
        }
    }

    #[test]
    fn observer_args_follow_the_binding_config_rules() {
        let set = SubscriberSet::new(EventKind::Activity);
        set.add_observer(observer("o_null"), Value::Null).unwrap();
        set.add_observer(observer("o_s"), json!({ "session_id": "c1" }))
            .unwrap();
        let err = set
            .add_observer(observer("o_bad"), json!({ "sesion_id": "c1" }))
            .unwrap_err();
        assert!(err.contains("computer::activity"), "{err}");
        assert_eq!(set.observer_snapshot().len(), 2);
        assert!(set.snapshot().is_empty(), "observers are not bindings");
        set.remove_observer("o_s");
        assert_eq!(set.observer_snapshot().len(), 1);
    }

    #[tokio::test]
    async fn an_observer_receives_the_event_a_binding_receives() {
        let sets = TriggerSets::new();
        let set = sets.for_kind(EventKind::SessionStopped);
        set.add(trigger_config(json!({ "session_id": "c1" })))
            .unwrap();
        let matching = observer("o1");
        let all = observer("o_all");
        let elsewhere = observer("o2");
        set.add_observer(matching.clone(), json!({ "session_id": "c1" }))
            .unwrap();
        set.add_observer(all.clone(), json!({})).unwrap();
        set.add_observer(elsewhere.clone(), json!({ "session_id": "c2" }))
            .unwrap();
        // Another kind's observer never sees this event.
        let other_kind = observer("o_started");
        sets.for_kind(EventKind::SessionStarted)
            .add_observer(other_kind.clone(), json!({}))
            .unwrap();
        let bindings = Arc::new(PayloadRecorder(Mutex::new(Vec::new())));
        let emitter = Emitter::new(sets, bindings.clone());

        let event = json!({ "session_id": "c1", "reason": "stopped", "timestamp": 1 });
        emitter.emit(EventKind::SessionStopped, "c1", &event).await;

        let bound = bindings.0.lock().unwrap().clone();
        assert_eq!(
            bound,
            vec![("console::on-event".to_string(), event.clone())]
        );
        assert_eq!(*matching.seen.lock().unwrap(), vec![event.clone()]);
        assert_eq!(*all.seen.lock().unwrap(), vec![event]);
        assert!(elsewhere.seen.lock().unwrap().is_empty());
        assert!(other_kind.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_observer_alone_gets_the_event_and_none_after_removal() {
        let sets = TriggerSets::new();
        let watcher = observer("o1");
        let activity = sets.for_kind(EventKind::Activity).clone();
        activity.add_observer(watcher.clone(), Value::Null).unwrap();
        let bindings = Arc::new(PayloadRecorder(Mutex::new(Vec::new())));
        let emitter = Emitter::new(sets, bindings.clone());

        let event = json!({ "session_id": "c9", "function": "computer::act", "timestamp": 2 });
        emitter.emit(EventKind::Activity, "c9", &event).await;
        assert_eq!(*watcher.seen.lock().unwrap(), vec![event.clone()]);
        assert!(bindings.0.lock().unwrap().is_empty());

        activity.remove_observer("o1");
        emitter.emit(EventKind::Activity, "c9", &event).await;
        assert_eq!(watcher.seen.lock().unwrap().len(), 1);
    }
}
