//! The custom trigger types this worker emits, and the fan-out behind
//! them. Consumers bind handlers with the standard two-step pattern; the
//! engine routes each registration to our [`TriggerHandler`]. Delivery is
//! fire-and-forget (`TriggerAction::Void`) and at-least-once; per-binding
//! `config` filters are evaluated here by the emitting worker, and malformed
//! configs are rejected at registration time.

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

pub const SESSION_STARTED: &str = "browser::session-started";
pub const SESSION_STOPPED: &str = "browser::session-stopped";
pub const NAVIGATED: &str = "browser::navigated";
pub const CONSOLE_EVENT: &str = "browser::console-event";
pub const NETWORK_EVENT: &str = "browser::network-event";
pub const PICKED: &str = "browser::picked";
pub const HANDOFF_REQUESTED: &str = "browser::handoff-requested";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    SessionStarted,
    SessionStopped,
    Navigated,
    ConsoleEvent,
    NetworkEvent,
    Picked,
    HandoffRequested,
}

impl EventKind {
    pub fn trigger_type(&self) -> &'static str {
        match self {
            EventKind::SessionStarted => SESSION_STARTED,
            EventKind::SessionStopped => SESSION_STOPPED,
            EventKind::Navigated => NAVIGATED,
            EventKind::ConsoleEvent => CONSOLE_EVENT,
            EventKind::NetworkEvent => NETWORK_EVENT,
            EventKind::Picked => PICKED,
            EventKind::HandoffRequested => HANDOFF_REQUESTED,
        }
    }

    pub fn all() -> [EventKind; 7] {
        [
            EventKind::SessionStarted,
            EventKind::SessionStopped,
            EventKind::Navigated,
            EventKind::ConsoleEvent,
            EventKind::NetworkEvent,
            EventKind::Picked,
            EventKind::HandoffRequested,
        ]
    }
}

/// Config accepted by every `browser::*` trigger binding. The only filter is
/// an optional session-id equality match; unknown fields fail at registration
/// so a misspelled filter key fails loudly instead of silently receiving
/// nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingConfig {
    /// Only deliver events for this browser session.
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

/// `browser::session-started` — a Chromium session is up and ready.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStartedEvent {
    pub session_id: String,
    pub url: String,
    pub headless: bool,
    pub timestamp: i64,
}

/// `browser::session-stopped` — a session ended; see `reason`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStoppedEvent {
    pub session_id: String,
    /// One of `stopped`, `idle`, `crashed`.
    pub reason: String,
    pub timestamp: i64,
}

/// `browser::navigated` — the session's page committed a navigation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NavigatedEvent {
    pub session_id: String,
    pub url: String,
    pub timestamp: i64,
}

/// `browser::console-event` — one console/log/exception entry was captured.
/// High-volume: bind with a `session_id` filter and debounce on the consumer
/// side; the durable record is the ring buffer behind `browser::console::read`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsoleEventPayload {
    pub session_id: String,
    pub entry: crate::session::ConsoleEntry,
}

/// `browser::network-event` — one network request was captured (completed or
/// failed). Symmetric with `browser::console-event`; the durable record is the
/// ring buffer behind `browser::network::read`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NetworkEventPayload {
    pub session_id: String,
    pub entry: crate::session::NetworkEntry,
}

/// `browser::picked` — the human picked an element in inspect mode
/// (`browser::pick::start`). Carries everything a chat composer needs to
/// describe the element, plus a `ref` that `browser::act` and
/// `browser::evaluate` accept directly.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PickedEvent {
    pub session_id: String,
    pub element: PickedElement,
    pub timestamp: i64,
}

/// `browser::handoff-requested` — a session is paused waiting for a human to
/// complete a step (CAPTCHA, 2FA, payment). The console surfaces this beside
/// the live viewport; resolve it with `browser::handoff::confirm` (or the
/// human clicks the in-page continue control).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HandoffRequestedEvent {
    pub session_id: String,
    pub handoff_id: String,
    /// What the human must do before the paused call continues.
    pub instructions: String,
    pub timestamp: i64,
}

/// The element payload inside `browser::picked`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PickedElement {
    /// Stable ref usable with `browser::act` / `browser::evaluate` until the
    /// next navigation.
    pub r#ref: String,
    /// Lowercase tag name (`button`, `input`, …).
    pub tag: String,
    /// Flattened `name="value"` attribute pairs.
    pub attributes: HashMap<String, String>,
    /// Outer HTML, truncated.
    pub outer_html: String,
    /// Trimmed innerText, truncated.
    pub text: String,
    /// Viewport-space bounding box.
    pub bounds: Bounds,
    /// Page URL at pick time.
    pub url: String,
    /// Most recent console errors at pick time (newest last).
    pub console_recent: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
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

/// The subscriber sets, one per trigger type.
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

struct BrowserTriggerHandler {
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for BrowserTriggerHandler {
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
    let descriptions: [(EventKind, &str); 7] = [
        (
            EventKind::SessionStarted,
            "A Chromium session is up and ready.",
        ),
        (
            EventKind::SessionStopped,
            "A Chromium session ended (stopped, idle, or crashed).",
        ),
        (
            EventKind::Navigated,
            "The session's page committed a navigation.",
        ),
        (
            EventKind::ConsoleEvent,
            "A console/log/exception entry was captured on a session's page.",
        ),
        (
            EventKind::NetworkEvent,
            "A network request was captured (completed or failed) on a session's page.",
        ),
        (
            EventKind::Picked,
            "The human picked an element in inspect mode.",
        ),
        (
            EventKind::HandoffRequested,
            "A session is paused waiting for a human to complete a step (CAPTCHA, 2FA, payment).",
        ),
    ];
    for (kind, description) in descriptions {
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                kind.trigger_type(),
                description,
                BrowserTriggerHandler {
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

/// How a matched event reaches a subscriber. Production delivers over the
/// bus; tests record.
#[async_trait]
pub trait EventDeliverer: Send + Sync {
    async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value);
}

/// Fire-and-forget bus delivery so the page-event pump that produced the
/// event is never blocked on subscriber latency. Failures are logged and
/// swallowed.
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

/// Evaluates each binding's session filter against each event and delivers
/// the payload to every match.
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
        // Filter before serializing: on the high-volume console-event pump,
        // a session with no matching binding must not pay the payload
        // serialization cost (a console panel open on session A means every
        // session-B log line would otherwise serialize for nothing).
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
        assert!(binding_matches(&BindingConfig::default(), "b1"));
    }

    #[test]
    fn session_filter_is_equality() {
        let filter = BindingConfig {
            session_id: Some("b1".to_string()),
        };
        assert!(binding_matches(&filter, "b1"));
        assert!(!binding_matches(&filter, "b2"));
    }

    #[test]
    fn add_rejects_unknown_filter_keys() {
        let set = SubscriberSet::new(EventKind::ConsoleEvent);
        let err = set
            .add(trigger_config(json!({ "sesion_id": "b1" })))
            .unwrap_err();
        assert!(err.contains("browser::console-event"), "{err}");
    }

    #[test]
    fn add_accepts_null_config() {
        let set = SubscriberSet::new(EventKind::Picked);
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
        let set = sets.for_kind(EventKind::Navigated);
        set.add(TriggerConfig {
            id: "match".to_string(),
            function_id: "fn::match".to_string(),
            config: json!({ "session_id": "b1" }),
            metadata: None,
        })
        .unwrap();
        set.add(TriggerConfig {
            id: "other".to_string(),
            function_id: "fn::other".to_string(),
            config: json!({ "session_id": "b2" }),
            metadata: None,
        })
        .unwrap();

        let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
        let emitter = Emitter::new(sets, recorder.clone());
        emitter
            .emit(
                EventKind::Navigated,
                "b1",
                &NavigatedEvent {
                    session_id: "b1".to_string(),
                    url: "http://localhost:3000".to_string(),
                    timestamp: 0,
                },
            )
            .await;

        let seen = recorder.0.lock().unwrap().clone();
        assert_eq!(seen, vec![(NAVIGATED.to_string(), "fn::match".to_string())]);
    }
}
