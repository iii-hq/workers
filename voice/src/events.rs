//! The custom trigger types this worker emits, the observable `voice::on-*`
//! function next to each one, and the fan-out behind them.
//!
//! Old consumers bind a trigger type with the standard two-step pattern; the
//! engine routes each registration to our [`TriggerHandler`]. New consumers
//! observe the function (iii 0.25) with the same config as args; its
//! [`ObserveHandler`] files observers in the same subscriber set. Delivery is
//! fire-and-forget and at-least-once for bindings (`TriggerAction::Void`),
//! fire-and-forget `obs.call` for observers; the optional `session_id` /
//! `speech_id` filter is evaluated here by the emitting worker, and malformed
//! configs or args are rejected at registration time. Events are routed
//! in-process: invoking an on-* function is refused with FORBIDDEN, so nobody
//! can forge one.

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

use crate::models::Progress;

pub const TRANSCRIPT: &str = "voice::transcript";
pub const SESSION_STARTED: &str = "voice::session-started";
pub const SESSION_STOPPED: &str = "voice::session-stopped";
pub const MODEL_PROGRESS: &str = "voice::model-progress";
pub const SPEECH_ENDED: &str = "voice::speech-ended";

pub const ON_TRANSCRIPT: &str = "voice::on-transcript";
pub const ON_SESSION_STARTED: &str = "voice::on-session-started";
pub const ON_SESSION_STOPPED: &str = "voice::on-session-stopped";
pub const ON_MODEL_PROGRESS: &str = "voice::on-model-progress";
pub const ON_SPEECH_ENDED: &str = "voice::on-speech-ended";
pub const ON_TRANSCRIPT_DESC: &str =
    "A dictation session produced text. Observe it with args { session_id? }; each observer \
     receives { session_id, seq, kind: partial | final | closed | error, text, segment, \
     timestamp_ms, reason? }. Only the voice worker emits it: invoking it directly is refused.";
pub const ON_SESSION_STARTED_DESC: &str =
    "A dictation session is ready for audio. Observe it with args { session_id? }; each \
     observer receives { session_id, model, timestamp_ms }. Only the voice worker emits it: \
     invoking it directly is refused.";
pub const ON_SESSION_STOPPED_DESC: &str =
    "A dictation session ended. Observe it with args { session_id? }; each observer receives \
     { session_id, reason: stopped | discarded | idle | shutdown, timestamp_ms }. Only the \
     voice worker emits it: invoking it directly is refused.";
pub const ON_MODEL_PROGRESS_DESC: &str =
    "Bytes received while a speech model downloads, one event per megabyte and a final done \
     event. Observe it with args {} (a session_id filter never matches); each observer \
     receives { id, file, received_bytes, total_bytes, done, error? }. Only the voice worker \
     emits it: invoking it directly is refused.";
pub const ON_SPEECH_ENDED_DESC: &str =
    "Legacy: the end of a host read-aloud, kept for compatibility and no longer emitted. \
     Observe it with args { speech_id? }; each observer would receive { speech_id, reason: \
     ended | stopped | failed, timestamp_ms }. Invoking it directly is refused.";

/// Codes the `voice::on-*` functions refuse with.
pub mod codes {
    /// Invoking an on-* function directly.
    pub const FORBIDDEN: &str = "FORBIDDEN";
    /// An observer's args the trigger type would refuse as a config.
    pub const INVALID_ARGS: &str = "INVALID_ARGS";
}

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

    /// The observable function next to the trigger type.
    pub fn observable_function(&self) -> &'static str {
        match self {
            EventKind::Transcript => ON_TRANSCRIPT,
            EventKind::SessionStarted => ON_SESSION_STARTED,
            EventKind::SessionStopped => ON_SESSION_STOPPED,
            EventKind::ModelProgress => ON_MODEL_PROGRESS,
            EventKind::SpeechEnded => ON_SPEECH_ENDED,
        }
    }

    /// The description of [`Self::observable_function`].
    pub fn observable_description(&self) -> &'static str {
        match self {
            EventKind::Transcript => ON_TRANSCRIPT_DESC,
            EventKind::SessionStarted => ON_SESSION_STARTED_DESC,
            EventKind::SessionStopped => ON_SESSION_STOPPED_DESC,
            EventKind::ModelProgress => ON_MODEL_PROGRESS_DESC,
            EventKind::SpeechEnded => ON_SPEECH_ENDED_DESC,
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

/// Legacy `voice::speech-ended` payload retained for trigger compatibility.
/// No longer emitted: all engines return audio; each caller owns playback.
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

/// One observer of a `voice::on-*` function with its parsed args.
#[derive(Clone)]
pub struct ObserverBinding {
    pub observer: Arc<dyn EventObserver>,
    pub filter: BindingConfig,
}

/// Thread-safe subscriber registry for one trigger type: its bindings and the
/// observers of its `voice::on-*` function, kept apart by id.
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

    pub fn kind(&self) -> EventKind {
        self.kind
    }

    /// Parse + insert a binding. Rejects malformed configs at registration.
    pub fn add(&self, config: TriggerConfig) -> Result<(), String> {
        self.add_binding(&config.id, &config.function_id, config.config)
    }

    /// A binding's config or an observer's args; null means no filter.
    fn parse_filter(&self, config: Value) -> Result<BindingConfig, String> {
        let raw = if config.is_null() {
            Value::Object(serde_json::Map::new())
        } else {
            config
        };
        serde_json::from_value(raw)
            .map_err(|e| format!("invalid {} binding config: {e}", self.kind.trigger_type()))
    }

    pub fn add_binding(&self, id: &str, function_id: &str, config: Value) -> Result<(), String> {
        let filter = self.parse_filter(config)?;
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

    /// Parse an observer's args (the trigger config) and file it. Malformed
    /// args reject the observer.
    pub fn add_observer(
        &self,
        observer: Arc<dyn EventObserver>,
        args: Value,
    ) -> Result<(), String> {
        let filter = self.parse_filter(args)?;
        let id = observer.id().to_string();
        self.observers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, ObserverBinding { observer, filter });
        Ok(())
    }

    pub fn remove_observer(&self, id: &str) {
        self.observers
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

    /// Observers whose args accept `session_id`.
    pub fn observers_matching(&self, session_id: Option<&str>) -> Vec<ObserverBinding> {
        self.observers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|o| binding_matches(&o.filter, self.kind, session_id))
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

/// One observer of a `voice::on-*` function, as the emitter sees it.
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

/// Files each observer of a `voice::on-*` function in the subscriber set its
/// trigger type uses, and takes it out again when the engine removes it.
struct VoiceObserveHandler {
    set: SubscriberSet,
}

#[async_trait]
impl ObserveHandler for VoiceObserveHandler {
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

/// The error every `voice::on-*` handler returns: events come only from this
/// worker, in-process, never by invoking the function.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: codes::FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args }}, target)) instead of invoking \
             it; only the voice worker emits its events"
        ),
        stacktrace: None,
    }
}

/// Draft-07 root schema of `T`, generated as iii-sdk does for a typed handler.
fn schema_for<T: JsonSchema>() -> Value {
    serde_json::to_value(
        schemars::r#gen::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<T>(),
    )
    .unwrap_or(Value::Null)
}

/// The request and response schemas `kind`'s observable function registers
/// with: the event its observers receive, and `()` (it never answers). Both
/// are set explicitly because the handler takes any JSON (see
/// [`observable_handler`]), which would otherwise publish an untyped request.
pub fn observable_formats(kind: EventKind) -> (Value, Value) {
    let request = match kind {
        EventKind::Transcript => schema_for::<TranscriptEvent>(),
        EventKind::SessionStarted => schema_for::<SessionStartedEvent>(),
        EventKind::SessionStopped => schema_for::<SessionStoppedEvent>(),
        EventKind::ModelProgress => schema_for::<Progress>(),
        EventKind::SpeechEnded => schema_for::<SpeechEndedEvent>(),
    };
    (request, schema_for::<()>())
}

/// Registers `kind`'s observable function, owner-delivered, with args
/// [`BindingConfig`] (the trigger config), the typed formats of
/// [`observable_formats`], and hooks filing observers in the trigger type's
/// subscriber set. On an engine without observe it registers as a plain
/// function and its hooks never run.
fn register_observable(iii: &IIIClient, kind: EventKind, sets: &TriggerSets) {
    let function_id = kind.observable_function();
    // The event type is only the documented request format: the handler takes
    // any JSON, so every invocation (empty or malformed input included) is
    // refused with FORBIDDEN rather than failing to deserialize first.
    let (request, response) = observable_formats(kind);
    iii.register_function(
        function_id,
        RegisterFunction::new(observable_handler(kind))
            .description(kind.observable_description())
            .request_format(request)
            .response_format(response)
            .observe(Observe::owner().args_format::<BindingConfig>().handler(
                VoiceObserveHandler {
                    set: sets.for_kind(kind).clone(),
                },
            )),
    );
    tracing::info!(function_id, "registered observable function");
}

/// The handler registered for `kind`'s observable function: takes any JSON
/// input and refuses it with [`refuse_invocation`] (FORBIDDEN).
pub fn observable_handler(
    kind: EventKind,
) -> impl Fn(Value) -> Result<(), Error> + Send + Sync + 'static {
    move |_input: Value| Err(refuse_invocation(kind.observable_function()))
}

/// Register the custom trigger types with the engine, and the observable
/// `voice::on-*` function next to each one, sharing one subscriber set per
/// type. Must run before
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
        // Kept for consumers that bind the trigger type, on 0.24 and 0.25
        // engines alike.
        #[allow(deprecated)]
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
        register_observable(iii, kind, &sets);
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
        let set = self.sets.for_kind(kind);
        for binding in set.matching(session_id) {
            self.deliverer
                .deliver(kind.trigger_type(), &binding.function_id, payload.clone())
                .await;
        }
        for observer in set.observers_matching(session_id) {
            observer.observer.deliver(payload.clone()).await;
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
    fn a_filter_on_the_other_kind_never_matches() {
        let dictation = BindingConfig {
            session_id: Some("d_1".into()),
            speech_id: None,
        };
        assert!(!binding_matches(
            &dictation,
            EventKind::SpeechEnded,
            Some("s_1")
        ));
        assert!(binding_matches(
            &dictation,
            EventKind::Transcript,
            Some("d_1")
        ));
        let speech = BindingConfig {
            session_id: None,
            speech_id: Some("s_1".into()),
        };
        assert!(!binding_matches(
            &speech,
            EventKind::Transcript,
            Some("d_1")
        ));
        assert!(binding_matches(
            &speech,
            EventKind::SpeechEnded,
            Some("s_1")
        ));
    }

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

    /// Stands in for the SDK observer: records what `obs.call` would carry
    /// to its target.
    struct ObserverRecorder {
        id: String,
        seen: Mutex<Vec<Value>>,
    }

    impl ObserverRecorder {
        fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.into(),
                seen: Mutex::new(Vec::new()),
            })
        }

        fn seen(&self) -> Vec<Value> {
            self.seen.lock().unwrap().clone()
        }
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

    #[test]
    fn each_on_function_refuses_direct_invocation() {
        for kind in EventKind::all() {
            let function_id = kind.observable_function();
            match refuse_invocation(function_id) {
                Error::Remote { code, message, .. } => {
                    assert_eq!(code, codes::FORBIDDEN);
                    assert_eq!(code, iii_sdk::protocol::observe_codes::FORBIDDEN);
                    assert!(message.contains(function_id), "{message}");
                }
                other => panic!("expected FORBIDDEN, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_registered_handler_refuses_empty_and_malformed_input_with_forbidden() {
        // `observable_handler(kind)` is exactly what `register_observable`
        // hands to RegisterFunction::new: it takes a serde_json::Value, so the
        // SDK's input deserialization cannot fail first (no Error::Serde) and
        // every input reaches the FORBIDDEN refusal.
        for kind in EventKind::all() {
            let handler = observable_handler(kind);
            for input in [
                serde_json::json!({}),
                Value::Null,
                serde_json::json!("not an event"),
                serde_json::json!([1, 2, 3]),
                serde_json::json!({ "session_id": 7, "seq": "x" }),
                serde_json::json!({
                    "session_id": "s1",
                    "seq": 1,
                    "kind": "final",
                    "text": "hi",
                    "segment": 0,
                    "timestamp_ms": 1
                }),
            ] {
                match handler(input.clone()) {
                    Err(Error::Remote { code, message, .. }) => {
                        assert_eq!(code, codes::FORBIDDEN, "{kind:?} {input}");
                        assert!(message.contains(kind.observable_function()), "{message}");
                    }
                    other => panic!("{kind:?} {input}: expected FORBIDDEN, got {other:?}"),
                }
            }
        }
    }

    #[test]
    fn function_ids_sit_next_to_their_trigger_types() {
        let pairs: Vec<(&str, &str)> = EventKind::all()
            .iter()
            .map(|kind| (kind.trigger_type(), kind.observable_function()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("voice::transcript", "voice::on-transcript"),
                ("voice::session-started", "voice::on-session-started"),
                ("voice::session-stopped", "voice::on-session-stopped"),
                ("voice::model-progress", "voice::on-model-progress"),
                ("voice::speech-ended", "voice::on-speech-ended"),
            ]
        );
        for kind in EventKind::all() {
            assert!(
                kind.observable_description().contains("refused"),
                "{}",
                kind.observable_function()
            );
        }
    }

    /// The interface boot smoke fails any function whose request or response
    /// schema is untyped (AnyValue or empty).
    fn assert_typed(function_id: &str, which: &str, schema: &Value) {
        let typed = schema.as_object().is_some_and(|o| {
            [
                "type",
                "properties",
                "$ref",
                "allOf",
                "anyOf",
                "oneOf",
                "enum",
            ]
            .iter()
            .any(|k| o.contains_key(*k))
        });
        assert!(typed, "{function_id} {which} schema is untyped: {schema}");
    }

    #[test]
    fn each_on_function_registers_typed_request_and_response_schemas() {
        for kind in EventKind::all() {
            let function_id = kind.observable_function();
            let (request, response) = observable_formats(kind);
            assert_typed(function_id, "request", &request);
            assert_typed(function_id, "response", &response);
            assert_eq!(response["type"], "null", "{function_id}");
            assert!(
                request["properties"]
                    .as_object()
                    .is_some_and(|p| !p.is_empty()),
                "{function_id}: the request names the event's fields: {request}"
            );
        }
    }

    #[tokio::test]
    async fn observer_receives_the_event_a_binding_receives() {
        for kind in EventKind::all() {
            let sets = TriggerSets::new();
            let set = sets.for_kind(kind);
            let field = if kind == EventKind::SpeechEnded {
                "speech_id"
            } else {
                "session_id"
            };
            let config = serde_json::json!({ field: "s1" });
            set.add_binding("b1", "fn-b1", config.clone()).unwrap();
            let matching = ObserverRecorder::new("o1");
            let every = ObserverRecorder::new("o2");
            let other = ObserverRecorder::new("o3");
            set.add_observer(matching.clone(), config).unwrap();
            set.add_observer(every.clone(), Value::Null).unwrap();
            set.add_observer(other.clone(), serde_json::json!({ field: "s2" }))
                .unwrap();

            let recorder = Arc::new(RecordingDeliverer::default());
            let emitter = Emitter::new(sets.clone(), recorder.clone());
            let event = serde_json::json!({ field: "s1", "timestamp_ms": 1 });
            emitter.emit(kind, Some("s1"), &event).await;

            let deliveries = recorder.deliveries.lock().unwrap().clone();
            assert_eq!(deliveries.len(), 1, "{kind:?}");
            assert_eq!(deliveries[0].0, kind.trigger_type());
            assert_eq!(deliveries[0].1, "fn-b1");
            let bound = deliveries[0].2.clone();
            assert_eq!(bound, event);
            assert_eq!(matching.seen(), vec![bound.clone()], "{kind:?}");
            assert_eq!(every.seen(), vec![bound], "{kind:?}");
            assert!(other.seen().is_empty(), "{kind:?}");
        }
    }

    #[tokio::test]
    async fn observers_follow_the_binding_filter_rules() {
        let sets = TriggerSets::new();
        let set = sets.for_kind(EventKind::Transcript);
        // A filter on the other kind's field never matches, as for a binding.
        let stray = ObserverRecorder::new("stray");
        set.add_observer(stray.clone(), serde_json::json!({ "speech_id": "s1" }))
            .unwrap();
        // A session filter does not match an event without a session.
        let session = ObserverRecorder::new("session");
        set.add_observer(session.clone(), serde_json::json!({ "session_id": "s1" }))
            .unwrap();
        assert!(set.observers_matching(Some("s1")).len() == 1);
        assert!(set.observers_matching(None).is_empty());

        // Malformed args are refused like a malformed binding config.
        let err = set
            .add_observer(
                ObserverRecorder::new("bad"),
                serde_json::json!({ "sesion_id": "x" }),
            )
            .unwrap_err();
        assert!(err.contains("unknown field"), "{err}");
        assert!(set
            .add_binding("bad", "fn-bad", serde_json::json!({ "sesion_id": "x" }))
            .is_err());

        let emitter = Emitter::new(sets.clone(), Arc::new(NoopDeliverer));
        emitter
            .emit(
                EventKind::Transcript,
                Some("s1"),
                &serde_json::json!({ "seq": 1 }),
            )
            .await;
        assert!(stray.seen().is_empty());
        assert_eq!(session.seen().len(), 1);
    }

    #[tokio::test]
    async fn a_removed_observer_stops_receiving_and_bindings_stay() {
        let sets = TriggerSets::new();
        let set = sets.for_kind(EventKind::SessionStarted);
        set.add_binding("same", "fn-same", Value::Null).unwrap();
        let observer = ObserverRecorder::new("same");
        set.add_observer(observer.clone(), Value::Null).unwrap();
        // Observer and binding ids live apart: removing one keeps the other.
        set.remove_observer("same");
        assert_eq!(set.len(), 1);
        assert!(set.observers_matching(None).is_empty());

        let recorder = Arc::new(RecordingDeliverer::default());
        Emitter::new(sets.clone(), recorder.clone())
            .emit(
                EventKind::SessionStarted,
                Some("d1"),
                &serde_json::json!({}),
            )
            .await;
        assert!(observer.seen().is_empty());
        assert_eq!(recorder.deliveries.lock().unwrap().len(), 1);

        // Observers of one type never receive another type's events.
        let started = ObserverRecorder::new("o1");
        set.add_observer(started.clone(), Value::Null).unwrap();
        Emitter::new(sets.clone(), Arc::new(NoopDeliverer))
            .emit(
                EventKind::SessionStopped,
                Some("d1"),
                &serde_json::json!({}),
            )
            .await;
        assert!(started.seen().is_empty());
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
