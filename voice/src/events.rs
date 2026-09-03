//! The custom trigger types this worker emits, and the fan-out behind them.
//!
//! Consumers bind handlers with the standard two-step pattern; the engine
//! routes each registration to our [`TriggerHandler`]. Delivery is
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

pub const TRANSCRIPT: &str = "voice::transcript";
pub const SESSION_STARTED: &str = "voice::session-started";
pub const SESSION_STOPPED: &str = "voice::session-stopped";
pub const MODEL_PROGRESS: &str = "voice::model-progress";
pub const SPEECH_ENDED: &str = "voice::speech-ended";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    Transcript,
    SessionStarted,
    SessionStopped,
    ModelProgress,
    SpeechEnded,
}

impl EventKind {
    pub fn trigger_type(&self) -> &'static str {
        match self {
            EventKind::Transcript => TRANSCRIPT,
            EventKind::SessionStarted => SESSION_STARTED,
            EventKind::SessionStopped => SESSION_STOPPED,
            EventKind::ModelProgress => MODEL_PROGRESS,
            EventKind::SpeechEnded => SPEECH_ENDED,
        }
    }

    pub fn all() -> [EventKind; 5] {
        [
            EventKind::Transcript,
            EventKind::SessionStarted,
            EventKind::SessionStopped,
            EventKind::ModelProgress,
            EventKind::SpeechEnded,
        ]
    }
}

/// Config accepted by every `voice::*` trigger binding. The only filter is an
/// optional id equality match: `session_id` for dictation events,
/// `speech_id` for `voice::speech-ended`. Unknown fields fail at
/// registration so a misspelled filter key fails loudly instead of silently
/// receiving nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingConfig {
    /// Only deliver events for this dictation session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Only deliver the end of this playback (`voice::speech-ended`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech_id: Option<String>,
}

/// `id` is the session id for dictation events and the speech id for
/// `voice::speech-ended`; each kind consults its own filter field. A
/// filter on the other kind's field can never match, so a binding that
/// sets it receives nothing rather than everything.
pub fn binding_matches(filter: &BindingConfig, kind: EventKind, id: Option<&str>) -> bool {
    let (want, stray) = match kind {
        EventKind::SpeechEnded => (&filter.speech_id, &filter.session_id),
        _ => (&filter.session_id, &filter.speech_id),
    };
    if stray.is_some() {
        return false;
    }
    match (want, id) {
        (Some(want), Some(have)) => want == have,
        (Some(_), None) => false,
        (None, _) => true,
    }
}

/// What a transcript event says about the session's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptKind {
    /// Replaces the in-progress text of the current segment.
    Partial,
    /// Commits a segment; the in-progress text is empty again.
    Final,
    /// The session ended; no more events follow.
    Closed,
    /// Something went wrong; see `reason`. The session may keep running.
    Error,
}

/// `voice::transcript` — one step of a dictation session's text.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TranscriptEvent {
    pub session_id: String,
    /// Monotonic per session; a receiver drops anything not newer than what
    /// it has seen.
    pub seq: u64,
    pub kind: TranscriptKind,
    pub text: String,
    /// Index of the segment this event belongs to.
    pub segment: u32,
    pub timestamp_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `voice::session-started` — a dictation session is ready for audio.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStartedEvent {
    pub session_id: String,
    pub model: String,
    pub timestamp_ms: i64,
}

/// `voice::session-stopped` — a session ended; see `reason`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStoppedEvent {
    pub session_id: String,
    /// One of `stopped`, `discarded`, `idle`, `shutdown`.
    pub reason: String,
    pub timestamp_ms: i64,
}

/// `voice::speech-ended` — a host playback started by `voice::speak` is
/// over. Fires for the `host` engine only: the other engines hand the audio
/// to the caller, who plays it and knows when it ends.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpeechEndedEvent {
    pub speech_id: String,
    /// `ended` (played to the end), `stopped` (voice::speak::stop), or
    /// `failed` (the speech command exited with an error).
    pub reason: String,
    pub timestamp_ms: i64,
}

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

    pub fn kind(&self) -> EventKind {
        self.kind
    }

    /// Parse + insert a binding. Rejects malformed configs at registration.
    pub fn add(&self, config: TriggerConfig) -> Result<(), String> {
        self.add_binding(&config.id, &config.function_id, config.config)
    }

    pub fn add_binding(&self, id: &str, function_id: &str, config: Value) -> Result<(), String> {
        let raw = if config.is_null() {
            Value::Object(serde_json::Map::new())
        } else {
            config
        };
        let filter: BindingConfig = serde_json::from_value(raw)
            .map_err(|e| format!("invalid {} binding config: {e}", self.kind.trigger_type()))?;
        let binding = Binding {
            id: id.to_string(),
            function_id: function_id.to_string(),
            filter,
        };
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.to_string(), binding);
        Ok(())
    }

    pub fn remove(&self, id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bindings whose filter accepts `session_id`.
    pub fn matching(&self, session_id: Option<&str>) -> Vec<Binding> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|b| binding_matches(&b.filter, self.kind, session_id))
            .cloned()
            .collect()
    }
}

/// One subscriber set per trigger type.
#[derive(Clone)]
pub struct TriggerSets {
    sets: HashMap<EventKind, SubscriberSet>,
}

impl Default for TriggerSets {
    fn default() -> Self {
        Self::new()
    }
}

impl TriggerSets {
    pub fn new() -> Self {
        Self {
            sets: EventKind::all()
                .into_iter()
                .map(|k| (k, SubscriberSet::new(k)))
                .collect(),
        }
    }

    pub fn for_kind(&self, kind: EventKind) -> &SubscriberSet {
        self.sets.get(&kind).expect("every kind is registered")
    }
}

struct VoiceTriggerHandler {
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for VoiceTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        self.set.add(config).map_err(Error::Handler)?;
        tracing::info!(id = %id, function_id = %function_id, trigger_type = self.set.kind().trigger_type(), "trigger subscription registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(id = %config.id, trigger_type = self.set.kind().trigger_type(), "trigger subscription unregistered");
        self.set.remove(&config.id);
        Ok(())
    }
}

/// Register the custom trigger types with the engine. Must run before
/// `functions::register_all` so handlers can capture the subscriber sets.
pub fn register_trigger_types(iii: &Arc<IIIClient>) -> TriggerSets {
    let sets = TriggerSets::new();
    let descriptions: [(EventKind, &str); 5] = [
        (
            EventKind::Transcript,
            "A dictation session produced text: a partial replaces the in-progress text, a \
             final commits a segment, closed ends the session. Filter with session_id.",
        ),
        (
            EventKind::SessionStarted,
            "A dictation session is ready for audio.",
        ),
        (
            EventKind::SessionStopped,
            "A dictation session ended (stopped, discarded, idle, or worker shutdown).",
        ),
        (
            EventKind::ModelProgress,
            "Bytes received while a speech model downloads, one event per megabyte and a \
             final done event (with error when the download failed).",
        ),
        (
            EventKind::SpeechEnded,
            "A host read-aloud started by voice::speak finished: reason ended, stopped or \
             failed. Filter with speech_id.",
        ),
    ];
    for (kind, description) in descriptions {
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                kind.trigger_type(),
                description,
                VoiceTriggerHandler {
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
                tracing::debug!(trigger_type, function_id, error = %e, "event delivery failed");
            }
        });
    }
}

/// Fan an event out to every matching binding of its kind.
pub struct Emitter {
    sets: TriggerSets,
    deliverer: Arc<dyn EventDeliverer>,
}

impl Emitter {
    pub fn new(sets: TriggerSets, deliverer: Arc<dyn EventDeliverer>) -> Self {
        Self { sets, deliverer }
    }

    pub async fn emit<T: Serialize>(&self, kind: EventKind, session_id: Option<&str>, event: &T) {
        let payload = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "event serialization failed");
                return;
            }
        };
        for binding in self.sets.for_kind(kind).matching(session_id) {
            self.deliverer
                .deliver(kind.trigger_type(), &binding.function_id, payload.clone())
                .await;
        }
    }

    /// Deliver a payload straight to one function, bypassing subscriptions
    /// (the dictation session's `output_function_id`).
    pub async fn deliver_direct<T: Serialize>(&self, function_id: &str, event: &T) {
        let payload = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "event serialization failed");
                return;
            }
        };
        self.deliverer
            .deliver(TRANSCRIPT, function_id, payload)
            .await;
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;

    /// Records every delivery, for tests.
    #[derive(Default)]
    pub struct RecordingDeliverer {
        pub deliveries: Mutex<Vec<(String, String, Value)>>,
    }

    #[async_trait]
    impl EventDeliverer for RecordingDeliverer {
        async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value) {
            self.deliveries.lock().unwrap().push((
                trigger_type.to_string(),
                function_id.to_string(),
                payload,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::RecordingDeliverer;
    use super::*;

    #[test]
    fn a_misspelled_filter_key_is_rejected() {
        let set = SubscriberSet::new(EventKind::Transcript);
        let err = set
            .add_binding("a", "fn-a", serde_json::json!({ "sesion_id": "x" }))
            .unwrap_err();
        assert!(err.contains("unknown field"), "{err}");
        assert!(set.is_empty());
    }

    #[test]
    fn filters_match_by_session() {
        let set = SubscriberSet::new(EventKind::Transcript);
        set.add_binding("all", "fn-all", Value::Null).unwrap();
        set.add_binding("one", "fn-one", serde_json::json!({ "session_id": "s1" }))
            .unwrap();
        assert_eq!(set.matching(Some("s1")).len(), 2);
        assert_eq!(set.matching(Some("s2")).len(), 1);
        assert_eq!(set.matching(None).len(), 1);
    }

    #[tokio::test]
    async fn emit_reaches_every_matching_binding() {
        let sets = TriggerSets::new();
        sets.for_kind(EventKind::SessionStopped)
            .add_binding("x", "fn-x", serde_json::json!({ "session_id": "s1" }))
            .unwrap();
        sets.for_kind(EventKind::SessionStopped)
            .add_binding("y", "fn-y", Value::Null)
            .unwrap();
        let recorder = Arc::new(RecordingDeliverer::default());
        let emitter = Emitter::new(sets, recorder.clone());
        emitter
            .emit(
                EventKind::SessionStopped,
                Some("s2"),
                &SessionStoppedEvent {
                    session_id: "s2".into(),
                    reason: "stopped".into(),
                    timestamp_ms: 1,
                },
            )
            .await;
        let deliveries = recorder.deliveries.lock().unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].1, "fn-y");
        assert_eq!(deliveries[0].0, SESSION_STOPPED);
    }
}

/// A deliverer that drops every event; for speakers built in tests.
pub struct NoopDeliverer;

#[async_trait]
impl EventDeliverer for NoopDeliverer {
    async fn deliver(&self, _trigger_type: &str, _function_id: &str, _payload: Value) {}
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
