//! The event functions this worker hosts, one per kind of browser event.
//! Each is private (only this worker calls it) and its handler returns the
//! event it is called with, so its observers observe the event. The worker
//! emits an event by calling its function with `CallAction::Void`; iii
//! delivers the call to every observer whose filters match, and the emitter
//! never waits for them.
//!
//! `browser::on-frame-event` takes `{ session_id? }` args: an observer with a
//! `session_id` receives that tab's frames only, and the frame pump encodes
//! nothing for a tab nobody watches. Every other function takes no args;
//! observers select a tab with a filter on `input.session_id`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{Call, CallAction, CallRequest, IIIClient, Keys, Match, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const ON_SESSION_STARTED_FN: &str = "browser::on-session-started";
pub const ON_SESSION_STOPPED_FN: &str = "browser::on-session-stopped";
pub const ON_SESSION_UPDATED_FN: &str = "browser::on-session-updated";
pub const ON_NAVIGATED_FN: &str = "browser::on-navigated";
pub const ON_CONSOLE_EVENT_FN: &str = "browser::on-console-event";
pub const ON_NETWORK_EVENT_FN: &str = "browser::on-network-event";
pub const ON_PICKED_FN: &str = "browser::on-picked";
pub const ON_HANDOFF_REQUESTED_FN: &str = "browser::on-handoff-requested";
pub const ON_HANDOFF_RESOLVED_FN: &str = "browser::on-handoff-resolved";
pub const ON_DOWNLOAD_CHANGED_FN: &str = "browser::on-download-changed";
pub const ON_FRAME_EVENT_FN: &str = "browser::on-frame-event";
pub const ON_CHROMIUM_INSTALL_PROGRESS_FN: &str = "browser::on-chromium-install-progress";

/// The key an observer of `browser::on-frame-event` without a `session_id`
/// is filed under: every frame call lists it.
const ALL_SESSIONS: &str = "*";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    SessionStarted,
    SessionStopped,
    SessionUpdated,
    Navigated,
    ConsoleEvent,
    NetworkEvent,
    Picked,
    HandoffRequested,
    HandoffResolved,
    DownloadChanged,
    FrameEvent,
    ChromiumInstallProgress,
}

impl EventKind {
    /// The function this worker calls to emit the event.
    pub fn function_id(&self) -> &'static str {
        match self {
            EventKind::SessionStarted => ON_SESSION_STARTED_FN,
            EventKind::SessionStopped => ON_SESSION_STOPPED_FN,
            EventKind::SessionUpdated => ON_SESSION_UPDATED_FN,
            EventKind::Navigated => ON_NAVIGATED_FN,
            EventKind::ConsoleEvent => ON_CONSOLE_EVENT_FN,
            EventKind::NetworkEvent => ON_NETWORK_EVENT_FN,
            EventKind::Picked => ON_PICKED_FN,
            EventKind::HandoffRequested => ON_HANDOFF_REQUESTED_FN,
            EventKind::HandoffResolved => ON_HANDOFF_RESOLVED_FN,
            EventKind::DownloadChanged => ON_DOWNLOAD_CHANGED_FN,
            EventKind::FrameEvent => ON_FRAME_EVENT_FN,
            EventKind::ChromiumInstallProgress => ON_CHROMIUM_INSTALL_PROGRESS_FN,
        }
    }

    pub fn all() -> [EventKind; 12] {
        [
            EventKind::SessionStarted,
            EventKind::SessionStopped,
            EventKind::SessionUpdated,
            EventKind::Navigated,
            EventKind::ConsoleEvent,
            EventKind::NetworkEvent,
            EventKind::Picked,
            EventKind::HandoffRequested,
            EventKind::HandoffResolved,
            EventKind::DownloadChanged,
            EventKind::FrameEvent,
            EventKind::ChromiumInstallProgress,
        ]
    }

    /// Whether observers choose the event's session with args
    /// (`{ session_id? }`) rather than with a filter.
    pub fn matched_by_session(&self) -> bool {
        matches!(self, EventKind::FrameEvent)
    }

    /// The registration description: what happened, and how to observe it.
    pub fn description(&self) -> &'static str {
        match self {
            EventKind::SessionStarted => {
                "A browser tab opened and is ready. Returns the SessionStartedEvent it is called \
                 with. No args; filter on input.session_id. Private: only the browser worker \
                 calls it."
            }
            EventKind::SessionStopped => {
                "A browser tab closed for good (stopped, idle, expired, or crashed). Returns the \
                 SessionStoppedEvent it is called with. No args; filter on input.session_id. \
                 Private: only the browser worker calls it."
            }
            EventKind::SessionUpdated => {
                "A browser tab woke up (page open again), went to sleep (page closed, tab kept), \
                 or its page's title changed. Returns the SessionUpdatedEvent it is called with. \
                 No args; filter on input.session_id. Private: only the browser worker calls it."
            }
            EventKind::Navigated => {
                "A tab's page committed a navigation. Returns the NavigatedEvent it is called \
                 with. No args; filter on input.session_id. Private: only the browser worker \
                 calls it."
            }
            EventKind::ConsoleEvent => {
                "A console/log/exception entry was captured on a tab's page (high volume). \
                 Returns the ConsoleEventPayload it is called with. No args; filter on \
                 input.session_id. Private: only the browser worker calls it."
            }
            EventKind::NetworkEvent => {
                "A network request was captured (completed or failed) on a tab's page. Returns \
                 the NetworkEventPayload it is called with. No args; filter on input.session_id. \
                 Private: only the browser worker calls it."
            }
            EventKind::Picked => {
                "The human picked an element in inspect mode. Returns the PickedEvent it is \
                 called with. No args; filter on input.session_id. Private: only the browser \
                 worker calls it."
            }
            EventKind::HandoffRequested => {
                "A tab is paused waiting for a human to complete a step (CAPTCHA, 2FA, payment). \
                 Returns the HandoffRequestedEvent it is called with. No args; filter on \
                 input.session_id. Private: only the browser worker calls it."
            }
            EventKind::HandoffResolved => {
                "A paused handoff finished (confirmed in page, by call, timed out, or \
                 cancelled). Returns the HandoffResolvedEvent it is called with. No args; filter \
                 on input.session_id. Private: only the browser worker calls it."
            }
            EventKind::DownloadChanged => {
                "A download started, progressed, or finished in a tab. Returns the \
                 DownloadChangedEvent it is called with. No args; filter on input.session_id. \
                 Private: only the browser worker calls it."
            }
            EventKind::FrameEvent => {
                "Internal: a live screencast frame of a watched tab (console viewport plumbing, \
                 high volume). Returns the FrameEventPayload it is called with. Args \
                 { session_id? }: an observer with a session_id receives that tab's frames only; \
                 observe with max_in_flight: 1 so a slow viewer skips frames. Private: only the \
                 browser worker calls it."
            }
            EventKind::ChromiumInstallProgress => {
                "A browser::chromium::install job moved: a new phase (resolving, downloading, \
                 extracting, verifying, done, failed) or about 1% more bytes. Returns the \
                 InstallProgress it is called with. No args; filter on input.job_id or \
                 input.phase. Private: only the browser worker calls it."
            }
        }
    }

    /// The function's registration: private, its handler returns the event
    /// it is called with, and its request and response format are the
    /// event's schema.
    fn registration(&self) -> RegisterFunction {
        let source = match self {
            EventKind::SessionStarted => event_source::<SessionStartedEvent>(),
            EventKind::SessionStopped => event_source::<SessionStoppedEvent>(),
            EventKind::SessionUpdated => event_source::<SessionUpdatedEvent>(),
            EventKind::Navigated => event_source::<NavigatedEvent>(),
            EventKind::ConsoleEvent => event_source::<ConsoleEventPayload>(),
            EventKind::NetworkEvent => event_source::<NetworkEventPayload>(),
            EventKind::Picked => event_source::<PickedEvent>(),
            EventKind::HandoffRequested => event_source::<HandoffRequestedEvent>(),
            EventKind::HandoffResolved => event_source::<HandoffResolvedEvent>(),
            EventKind::DownloadChanged => event_source::<DownloadChangedEvent>(),
            EventKind::FrameEvent => {
                event_source::<FrameEventPayload>().args_format::<FrameEventArgs>()
            }
            EventKind::ChromiumInstallProgress => event_source::<ChromiumInstallProgressEvent>(),
        };
        source.description(self.description())
    }
}

/// Args of a `browser::on-frame-event` observer. Unknown fields are refused
/// (`INVALID_ARGS`), so a misspelled key fails at `observe` instead of
/// receiving every tab's frames.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameEventArgs {
    /// Only this tab's frames. Without it, the observer receives every
    /// watched tab's frames.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// The JSON Schema of `T`, built the way iii-sdk builds a typed handler's
/// formats at registration.
fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(
        schemars::r#gen::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<T>(),
    )
    .unwrap_or(Value::Null)
}

/// A private function whose handler returns the event it is called with.
/// The handler passes the JSON through untouched (a frame is ~100 KB of
/// base64), and the formats carry the event's schema.
fn event_source<E: JsonSchema>() -> RegisterFunction {
    let schema = schema_of::<E>();
    RegisterFunction::new(|event: Value| Ok::<_, Error>(event))
        .request_format(schema.clone())
        .response_format(schema)
        .private()
}

/// `browser::on-frame-event`: an observer with a `session_id` is filed under
/// it, one without under [`ALL_SESSIONS`]; a frame matches both its tab's
/// key and [`ALL_SESSIONS`].
fn frame_match() -> Match {
    Match::keys(
        |obs| match obs.args_value().get("session_id").and_then(Value::as_str) {
            Some(session_id) => Keys::from(session_id),
            None => Keys::from(ALL_SESSIONS),
        },
        |call| match call.input.get("session_id").and_then(Value::as_str) {
            Some(session_id) => Keys::from(vec![session_id, ALL_SESSIONS]),
            None => Keys::from(ALL_SESSIONS),
        },
    )
}

/// Register the event functions. The frame function's match hook is set by
/// id first, so it files every observer.
pub fn register_event_functions(iii: &IIIClient) {
    iii.on_observer_match_for(ON_FRAME_EVENT_FN, frame_match());
    for kind in EventKind::all() {
        iii.register_function(kind.function_id(), kind.registration());
        tracing::debug!(
            function_id = kind.function_id(),
            "registered event function"
        );
    }
}

// ---------------------------------------------------------------------------
// Events (what each function returns, and its observers observe)
// ---------------------------------------------------------------------------

/// `browser::on-session-started` — a Chromium session is up and ready.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStartedEvent {
    pub session_id: String,
    pub url: String,
    pub headless: bool,
    /// False when the opener asked for no live preview (`preview: false`
    /// on `sessions::start`); consoles then show no thumbnail for it.
    pub preview: bool,
    pub timestamp: i64,
}

/// `browser::on-session-stopped` — a tab closed for good; see `reason`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStoppedEvent {
    pub session_id: String,
    /// One of `stopped`, `idle`, `expired`, `crashed`.
    pub reason: String,
    pub timestamp: i64,
}

/// `browser::on-session-updated` — a tab woke up (`active: true`, its page is
/// open again), went to sleep (`active: false`, page closed, tab kept), or
/// its live page's title changed (`active: true`; reported by an in-page
/// title watcher the page cannot see). The tab strip re-reads
/// `browser::sessions::list` on this; URL moves arrive as
/// `browser::on-navigated`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionUpdatedEvent {
    pub session_id: String,
    pub active: bool,
    pub url: String,
    pub title: String,
    pub timestamp: i64,
}

/// `browser::on-navigated` — the session's page committed a navigation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NavigatedEvent {
    pub session_id: String,
    pub url: String,
    pub timestamp: i64,
}

/// `browser::on-download-changed` — a download in the session started,
/// progressed, or finished. The durable record is `browser::downloads::list`;
/// this is the nudge to re-list.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DownloadChangedEvent {
    pub session_id: String,
    pub timestamp: i64,
}

/// `browser::on-frame-event` — one live screencast frame of a watched tab,
/// the picture the console viewport paints. Internal console-UI plumbing and
/// high-volume (up to 30 a second, base64 JPEG): observe with
/// `{ session_id }` args and `max_in_flight: 1` only while a viewer is on
/// screen. `browser::frame` reads the newest frame once for the first paint.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FrameEventPayload {
    pub session_id: String,
    /// Base64 JPEG.
    pub frame: String,
    /// Page-viewport size the frame maps to (input coordinate space).
    pub width: u32,
    pub height: u32,
    /// Monotonic per-tab cursor; a viewer ignores frames older than the last
    /// one it painted.
    pub frame_seq: u64,
    pub timestamp: i64,
}

/// `browser::on-console-event` — one console/log/exception entry was
/// captured. High-volume: observe with an `input.session_id` filter and
/// debounce on the consumer side; the durable record is the ring buffer
/// behind `browser::console::read`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsoleEventPayload {
    pub session_id: String,
    pub entry: crate::session::ConsoleEntry,
}

/// `browser::on-network-event` — one network request was captured (completed
/// or failed). Symmetric with `browser::on-console-event`; the durable record
/// is the ring buffer behind `browser::network::read`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NetworkEventPayload {
    pub session_id: String,
    pub entry: crate::session::NetworkEntry,
}

/// `browser::on-picked` — the human picked an element in inspect mode
/// (`browser::pick::start`). Carries everything a chat composer needs to
/// describe the element, plus a `ref` that `browser::act` and
/// `browser::evaluate` accept directly.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PickedEvent {
    pub session_id: String,
    pub element: PickedElement,
    pub timestamp: i64,
}

/// `browser::on-handoff-requested` — a session is paused waiting for a human to
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

/// `browser::on-handoff-resolved` — a paused handoff finished, however it
/// finished. The console clears its banner on this; `via` is `in_page`,
/// `confirm_call`, `timeout`, or `cancelled`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HandoffResolvedEvent {
    pub session_id: String,
    pub handoff_id: String,
    pub via: String,
    pub timestamp: i64,
}

/// `browser::on-chromium-install-progress` returns
/// [`crate::chromium::install::InstallProgress`]: `{ job_id, phase, version?,
/// bytes_done, bytes_total?, path?, error?, hint?, timestamp }`. It is not a
/// session event and carries no `session_id`.
pub use crate::chromium::install::InstallProgress as ChromiumInstallProgressEvent;

/// The element payload inside `browser::on-picked`.
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
// Emission
// ---------------------------------------------------------------------------

/// Where emitted events go: the event functions on the bus in production, a
/// recorder in tests.
#[async_trait]
pub trait EventSink: Send + Sync {
    /// Whether an event of `kind` for `session_id` would reach any observer.
    fn is_observed(&self, kind: EventKind, session_id: &str) -> bool;
    /// Call `kind`'s function with `event`. Returns without waiting for the
    /// observers.
    async fn call(&self, kind: EventKind, event: Value);
}

/// Calls the event functions on the bus with `CallAction::Void`. The
/// functions are this worker's own, so each call runs in-process and the
/// observers are reported to the engine without waiting for them. Failures
/// are logged and swallowed: an event is never worth failing the pump that
/// produced it.
pub struct IiiSink {
    iii: Arc<IIIClient>,
}

impl IiiSink {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl EventSink for IiiSink {
    fn is_observed(&self, kind: EventKind, session_id: &str) -> bool {
        if kind.matched_by_session() {
            let call = Call {
                input: json!({ "session_id": session_id }),
                ..Call::default()
            };
            self.iii.is_observed_for(kind.function_id(), &call)
        } else {
            self.iii.is_observed(kind.function_id())
        }
    }

    async fn call(&self, kind: EventKind, event: Value) {
        let res = self
            .iii
            .call(CallRequest {
                function_id: kind.function_id().to_string(),
                payload: event,
                action: Some(CallAction::Void),
                timeout_ms: None,
            })
            .await;
        if let Err(e) = res {
            tracing::warn!(function_id = kind.function_id(), error = %e, "event call failed");
        }
    }
}

/// Emits events: nothing is serialized or called for an event nobody
/// observes.
pub struct Emitter {
    sink: Arc<dyn EventSink>,
}

impl Emitter {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self { sink }
    }

    /// The production emitter: [`IiiSink`] over `iii`.
    pub fn for_iii(iii: Arc<IIIClient>) -> Self {
        Self::new(Arc::new(IiiSink::new(iii)))
    }

    /// Whether an event of `kind` for this session would reach anyone, so a
    /// producer can skip building it (copying a frame, say).
    pub fn is_observed(&self, kind: EventKind, session_id: &str) -> bool {
        self.sink.is_observed(kind, session_id)
    }

    /// Call `kind`'s function with `payload` when anyone observes it.
    /// `session_id` is the tab the event belongs to (`""` for an event of no
    /// tab); it picks the observers of a function matched by session.
    pub async fn emit<T: Serialize>(&self, kind: EventKind, session_id: &str, payload: &T) {
        // Check before serializing: on the high-volume console pump, an
        // unobserved session must not pay the payload serialization cost.
        if !self.sink.is_observed(kind, session_id) {
            return;
        }
        let event = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(function_id = kind.function_id(), error = %e, "event failed to serialize");
                return;
            }
        };
        self.sink.call(kind, event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Observes the kinds in `observed` (for the kind matched by session,
    /// only the sessions in `sessions`) and records every call.
    #[derive(Default)]
    struct Recorder {
        observed: Vec<EventKind>,
        sessions: Vec<String>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl EventSink for Recorder {
        fn is_observed(&self, kind: EventKind, session_id: &str) -> bool {
            self.observed.contains(&kind)
                && (!kind.matched_by_session() || self.sessions.iter().any(|s| s == session_id))
        }

        async fn call(&self, kind: EventKind, event: Value) {
            self.calls
                .lock()
                .unwrap()
                .push((kind.function_id().to_string(), event));
        }
    }

    fn navigated(session_id: &str) -> NavigatedEvent {
        NavigatedEvent {
            session_id: session_id.to_string(),
            url: "http://localhost:3000".to_string(),
            timestamp: 0,
        }
    }

    #[test]
    fn function_ids_are_the_on_form_of_each_event() {
        let ids: Vec<&str> = EventKind::all()
            .iter()
            .map(EventKind::function_id)
            .collect();
        assert_eq!(
            ids,
            [
                "browser::on-session-started",
                "browser::on-session-stopped",
                "browser::on-session-updated",
                "browser::on-navigated",
                "browser::on-console-event",
                "browser::on-network-event",
                "browser::on-picked",
                "browser::on-handoff-requested",
                "browser::on-handoff-resolved",
                "browser::on-download-changed",
                "browser::on-frame-event",
                "browser::on-chromium-install-progress",
            ]
        );
    }

    #[test]
    fn only_the_frame_function_takes_args() {
        for kind in EventKind::all() {
            assert_eq!(
                kind.matched_by_session(),
                kind == EventKind::FrameEvent,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn frame_args_refuse_unknown_keys() {
        let err = serde_json::from_value::<FrameEventArgs>(json!({ "sesion_id": "b1" }))
            .unwrap_err()
            .to_string();
        assert!(err.contains("sesion_id"), "{err}");
        let args: FrameEventArgs = serde_json::from_value(json!({})).unwrap();
        assert!(args.session_id.is_none());
        let schema = schema_of::<FrameEventArgs>();
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn event_schemas_are_typed() {
        let schema = schema_of::<NavigatedEvent>();
        assert_eq!(schema["type"], "object");
        let mut required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        required.sort_unstable();
        assert_eq!(required, ["session_id", "timestamp", "url"]);
    }

    #[tokio::test]
    async fn emit_calls_the_function_of_an_observed_event() {
        let recorder = Arc::new(Recorder {
            observed: vec![EventKind::Navigated],
            ..Recorder::default()
        });
        let emitter = Emitter::new(recorder.clone());
        emitter
            .emit(EventKind::Navigated, "b1", &navigated("b1"))
            .await;
        emitter
            .emit(
                EventKind::DownloadChanged,
                "b1",
                &DownloadChangedEvent {
                    session_id: "b1".to_string(),
                    timestamp: 0,
                },
            )
            .await;

        let calls = recorder.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "an unobserved event is not called");
        assert_eq!(calls[0].0, ON_NAVIGATED_FN);
        assert_eq!(calls[0].1["session_id"], "b1");
    }

    #[tokio::test]
    async fn frames_are_emitted_only_for_watched_sessions() {
        let recorder = Arc::new(Recorder {
            observed: vec![EventKind::FrameEvent],
            sessions: vec!["b1".to_string()],
            ..Recorder::default()
        });
        let emitter = Emitter::new(recorder.clone());
        assert!(emitter.is_observed(EventKind::FrameEvent, "b1"));
        assert!(!emitter.is_observed(EventKind::FrameEvent, "b2"));
        for session_id in ["b1", "b2"] {
            emitter
                .emit(
                    EventKind::FrameEvent,
                    session_id,
                    &FrameEventPayload {
                        session_id: session_id.to_string(),
                        frame: String::new(),
                        width: 1,
                        height: 1,
                        frame_seq: 1,
                        timestamp: 0,
                    },
                )
                .await;
        }
        let calls = recorder.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, ON_FRAME_EVENT_FN);
        assert_eq!(calls[0].1["session_id"], "b1");
    }
}
