//! The two custom trigger types this worker emits, and the fan-out behind
//! them. Consumers bind handlers with the standard two-step pattern; the
//! engine routes each registration to our [`TriggerHandler`]. Delivery is
//! fire-and-forget (`TriggerAction::Void`) and at-least-once; the optional
//! per-binding `session_id` filter is evaluated here by the emitting worker,
//! and malformed configs are rejected at registration time.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::Screen;

pub const SESSION_STARTED: &str = "computer::session-started";
pub const SESSION_STOPPED: &str = "computer::session-stopped";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    SessionStarted,
    SessionStopped,
}

impl EventKind {
    pub fn trigger_type(&self) -> &'static str {
        match self {
            EventKind::SessionStarted => SESSION_STARTED,
            EventKind::SessionStopped => SESSION_STOPPED,
        }
    }

    pub fn all() -> [EventKind; 2] {
        [EventKind::SessionStarted, EventKind::SessionStopped]
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
    /// The computer-server endpoint the session is driving.
    pub endpoint: String,
    /// Informational guest OS label (`linux`, `macos`, ...).
    pub os: String,
    pub screen: Screen,
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

/// Thread-safe subscriber registry for one trigger type.
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

    /// Parse + insert a binding. Rejects malformed configs at registration.
    pub fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let raw = if config.config.is_null() {
            Value::Object(serde_json::Map::new())
        } else {
            config.config.clone()
        };
        let filter: BindingConfig = serde_json::from_value(raw)
            .map_err(|e| format!("invalid {} config: {e}", self.kind.trigger_type()))?;
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

    /// Snapshot so the mutex is never held across awaits.
    pub fn snapshot(&self) -> Vec<Binding> {
        self.lock().values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Binding>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
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

/// Register the custom trigger types with the engine. Must run before
/// `functions::register_all` so handlers can capture the subscriber sets.
pub fn register_trigger_types(iii: &Arc<IIIClient>) -> TriggerSets {
    let sets = TriggerSets::new();
    let descriptions: [(EventKind, &str); 2] = [
        (
            EventKind::SessionStarted,
            "A desktop session connected and is ready to drive.",
        ),
        (
            EventKind::SessionStopped,
            "A desktop session ended (stopped or idle).",
        ),
    ];
    for (kind, description) in descriptions {
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
        let bindings = self.sets.for_kind(kind).snapshot();
        let mut matched = bindings
            .into_iter()
            .filter(|b| binding_matches(&b.filter, session_id))
            .peekable();
        if matched.peek().is_none() {
            return;
        }
        let payload = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "event payload failed to serialize");
                return;
            }
        };
        for binding in matched {
            self.deliverer
                .deliver(kind.trigger_type(), &binding.function_id, payload.clone())
                .await;
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
        })
        .unwrap();
        set.add(TriggerConfig {
            id: "other".to_string(),
            function_id: "fn::other".to_string(),
            config: json!({ "session_id": "c2" }),
            metadata: None,
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
}
