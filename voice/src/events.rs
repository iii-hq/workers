//! The event functions this worker provides, and the emitter that calls them.
//!
//! Each event is a private function whose handler returns the event it is
//! called with. The worker calls the function when the event happens,
//! fire-and-forget (`CallAction::Void`), and only while something observes it
//! (one atomic load); every observer whose filters match receives the call.
//! The functions take no args: consumers select events with filters on the
//! event's fields, `input.session_id` for dictation events.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::Progress;

pub const ON_TRANSCRIPT_FN: &str = "voice::on-transcript";
pub const ON_SESSION_STARTED_FN: &str = "voice::on-session-started";
pub const ON_SESSION_STOPPED_FN: &str = "voice::on-session-stopped";
pub const ON_MODEL_PROGRESS_FN: &str = "voice::on-model-progress";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    Transcript,
    SessionStarted,
    SessionStopped,
    ModelProgress,
}

impl EventKind {
    /// The function observers observe for this event.
    pub fn function_id(&self) -> &'static str {
        match self {
            EventKind::Transcript => ON_TRANSCRIPT_FN,
            EventKind::SessionStarted => ON_SESSION_STARTED_FN,
            EventKind::SessionStopped => ON_SESSION_STOPPED_FN,
            EventKind::ModelProgress => ON_MODEL_PROGRESS_FN,
        }
    }

    pub fn all() -> [EventKind; 4] {
        [
            EventKind::Transcript,
            EventKind::SessionStarted,
            EventKind::SessionStopped,
            EventKind::ModelProgress,
        ]
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

/// `voice::on-transcript` — one step of a dictation session's text.
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

/// `voice::on-session-started` — a dictation session is ready for audio.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStartedEvent {
    pub session_id: String,
    pub model: String,
    pub timestamp_ms: i64,
}

/// `voice::on-session-stopped` — a session ended; see `reason`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStoppedEvent {
    pub session_id: String,
    /// One of `stopped`, `discarded`, `idle`, `shutdown`.
    pub reason: String,
    pub timestamp_ms: i64,
}

/// Register the event functions. Each is private (only this worker calls it;
/// anyone may observe it) and its handler returns the event, so observers
/// observe the event itself.
pub fn register(iii: &IIIClient) {
    register_source::<TranscriptEvent>(
        iii,
        ON_TRANSCRIPT_FN,
        "A dictation session produced text: a partial replaces the in-progress text, a \
         final commits a segment, closed ends the session. Observe it with a filter on \
         input.session_id to follow one session.",
    );
    register_source::<SessionStartedEvent>(
        iii,
        ON_SESSION_STARTED_FN,
        "A dictation session is ready for audio. Filter on input.session_id.",
    );
    register_source::<SessionStoppedEvent>(
        iii,
        ON_SESSION_STOPPED_FN,
        "A dictation session ended (stopped, discarded, idle, or worker shutdown). Filter \
         on input.session_id.",
    );
    register_source::<Progress>(
        iii,
        ON_MODEL_PROGRESS_FN,
        "Bytes received while a speech model downloads, one event per megabyte and a \
         final done event (with error when the download failed). Filter on input.id.",
    );
}

fn register_source<E>(iii: &IIIClient, id: &str, description: &str)
where
    E: Serialize + DeserializeOwned + JsonSchema + Send + 'static,
{
    iii.register_function(
        id,
        RegisterFunction::new(|event: E| Ok::<_, Error>(event))
            .description(description)
            .private(),
    );
    tracing::info!(function_id = id, "registered event function");
}

/// How an event reaches iii. Production calls over the bus; tests record.
#[async_trait]
pub trait EventDeliverer: Send + Sync {
    /// Whether anything observes `function_id`.
    fn is_observed(&self, function_id: &str) -> bool;

    /// Call `function_id` with `payload`, fire-and-forget.
    async fn call(&self, function_id: &str, payload: Value);
}

/// Fire-and-forget bus delivery so the caller that produced the event is never
/// blocked on observer latency. Failures are logged and swallowed.
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
        let iii = self.iii.clone();
        let function_id = function_id.to_string();
        tokio::spawn(async move {
            let res = iii
                .call(CallRequest {
                    function_id: function_id.clone(),
                    payload,
                    action: Some(CallAction::Void),
                    timeout_ms: None,
                })
                .await;
            if let Err(e) = res {
                tracing::debug!(function_id, error = %e, "event delivery failed");
            }
        });
    }
}

/// Calls the event functions, and a dictation session's own output function.
pub struct Emitter {
    deliverer: Arc<dyn EventDeliverer>,
}

impl Emitter {
    pub fn new(deliverer: Arc<dyn EventDeliverer>) -> Self {
        Self { deliverer }
    }

    /// Call the event's function when anything observes it.
    pub async fn emit<T: Serialize>(&self, kind: EventKind, event: &T) {
        let function_id = kind.function_id();
        if !self.deliverer.is_observed(function_id) {
            return;
        }
        let payload = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "event serialization failed");
                return;
            }
        };
        self.deliverer.call(function_id, payload).await;
    }

    /// Call one function with the event, observed or not (the dictation
    /// session's `output_function_id`).
    pub async fn deliver_direct<T: Serialize>(&self, function_id: &str, event: &T) {
        let payload = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "event serialization failed");
                return;
            }
        };
        self.deliverer.call(function_id, payload).await;
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// Records every call, for tests. Reports every function as observed
    /// unless `observed` is false.
    pub struct RecordingDeliverer {
        pub observed: bool,
        pub calls: Mutex<Vec<(String, Value)>>,
    }

    impl Default for RecordingDeliverer {
        fn default() -> Self {
            Self {
                observed: true,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl EventDeliverer for RecordingDeliverer {
        fn is_observed(&self, _function_id: &str) -> bool {
            self.observed
        }

        async fn call(&self, function_id: &str, payload: Value) {
            self.calls
                .lock()
                .unwrap()
                .push((function_id.to_string(), payload));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::RecordingDeliverer;
    use super::*;

    fn stopped(session_id: &str) -> SessionStoppedEvent {
        SessionStoppedEvent {
            session_id: session_id.into(),
            reason: "stopped".into(),
            timestamp_ms: 1,
        }
    }

    #[test]
    fn event_functions_follow_the_on_naming() {
        let ids: Vec<_> = EventKind::all()
            .iter()
            .map(EventKind::function_id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "voice::on-transcript",
                "voice::on-session-started",
                "voice::on-session-stopped",
                "voice::on-model-progress",
            ]
        );
    }

    #[tokio::test]
    async fn emit_calls_the_event_function_with_the_event() {
        let recorder = Arc::new(RecordingDeliverer::default());
        let emitter = Emitter::new(recorder.clone());
        emitter
            .emit(EventKind::SessionStopped, &stopped("s2"))
            .await;
        let calls = recorder.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, ON_SESSION_STOPPED_FN);
        assert_eq!(calls[0].1["session_id"], "s2");
    }

    #[tokio::test]
    async fn emit_skips_an_unobserved_function() {
        let recorder = Arc::new(RecordingDeliverer {
            observed: false,
            ..RecordingDeliverer::default()
        });
        let emitter = Emitter::new(recorder.clone());
        emitter
            .emit(EventKind::SessionStopped, &stopped("s2"))
            .await;
        assert!(recorder.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn deliver_direct_calls_the_target_unobserved() {
        let recorder = Arc::new(RecordingDeliverer {
            observed: false,
            ..RecordingDeliverer::default()
        });
        let emitter = Emitter::new(recorder.clone());
        emitter
            .deliver_direct("my::transcript", &stopped("s1"))
            .await;
        let calls = recorder.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "my::transcript");
    }

    #[test]
    fn event_handlers_round_trip_the_event() {
        let event = TranscriptEvent {
            session_id: "d_1".into(),
            seq: 3,
            kind: TranscriptKind::Final,
            text: "hello".into(),
            segment: 0,
            timestamp_ms: 7,
            reason: None,
        };
        let value = serde_json::to_value(&event).unwrap();
        let back: TranscriptEvent = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), value);
    }
}

/// A deliverer that observes nothing and drops every call; for speakers
/// built in tests.
pub struct NoopDeliverer;

#[async_trait]
impl EventDeliverer for NoopDeliverer {
    fn is_observed(&self, _function_id: &str) -> bool {
        false
    }

    async fn call(&self, _function_id: &str, _payload: Value) {}
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
