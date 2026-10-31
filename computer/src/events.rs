//! The session events this worker provides, as observable functions. Each
//! is private (only this worker calls it), and its handler returns the event
//! it is called with: that event is what observers observe. None takes
//! args; observers select a session with a filter on `input.session_id`.
//! Calls are fire-and-forget (`CallAction::Void`), iii delivers them, and an
//! event nobody observes is never built.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::driver::Screen;

pub const ON_SESSION_STARTED_FN: &str = "computer::on-session-started";
pub const ON_SESSION_STOPPED_FN: &str = "computer::on-session-stopped";
pub const ON_ACTIVITY_FN: &str = "computer::on-activity";
pub const SESSION_STARTED_DESC: &str =
    "A desktop session connected and is ready to drive. No args; filter on input.session_id.";
pub const SESSION_STOPPED_DESC: &str =
    "A desktop session ended (stopped or idle). No args; filter on input.session_id.";
pub const ACTIVITY_DESC: &str =
    "A caller drove a desktop session (computer::act or computer::run); the console's live \
     preview shows that session. No args; filter on input.session_id.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    SessionStarted,
    SessionStopped,
    Activity,
}

impl EventKind {
    pub fn function_id(&self) -> &'static str {
        match self {
            EventKind::SessionStarted => ON_SESSION_STARTED_FN,
            EventKind::SessionStopped => ON_SESSION_STOPPED_FN,
            EventKind::Activity => ON_ACTIVITY_FN,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            EventKind::SessionStarted => SESSION_STARTED_DESC,
            EventKind::SessionStopped => SESSION_STOPPED_DESC,
            EventKind::Activity => ACTIVITY_DESC,
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

// ---------------------------------------------------------------------------
// Events (what the functions return, and observers observe)
// ---------------------------------------------------------------------------

/// What `computer::on-session-started` returns: a desktop session is
/// connected and ready.
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

/// What `computer::on-activity` returns: a caller drove a session. Called
/// when a `computer::act` call reaches it, or a `computer::run` passed its
/// checks and holds it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ActivityEvent {
    pub session_id: String,
    /// The function called: `computer::act` or `computer::run`.
    pub function: String,
    pub timestamp: i64,
}

/// What `computer::on-session-stopped` returns: a session ended; see
/// `reason`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStoppedEvent {
    pub session_id: String,
    /// One of `stopped`, `idle`.
    pub reason: String,
    pub timestamp: i64,
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register the three event functions. Runs before `functions::register_all`
/// so every emit has its function.
pub fn register(iii: &IIIClient) {
    iii.register_function(
        ON_SESSION_STARTED_FN,
        RegisterFunction::new(|event: SessionStartedEvent| Ok::<_, Error>(event))
            .description(SESSION_STARTED_DESC)
            .private(),
    );
    iii.register_function(
        ON_SESSION_STOPPED_FN,
        RegisterFunction::new(|event: SessionStoppedEvent| Ok::<_, Error>(event))
            .description(SESSION_STOPPED_DESC)
            .private(),
    );
    iii.register_function(
        ON_ACTIVITY_FN,
        RegisterFunction::new(|event: ActivityEvent| Ok::<_, Error>(event))
            .description(ACTIVITY_DESC)
            .private(),
    );
    for kind in EventKind::all() {
        tracing::info!(
            function_id = kind.function_id(),
            "registered event function"
        );
    }
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// Where an event goes. Production calls the function on the bus; tests
/// record.
#[async_trait]
pub trait EventSink: Send + Sync {
    /// Whether anything observes `function_id`.
    fn observed(&self, function_id: &str) -> bool;
    async fn call(&self, function_id: &str, payload: Value);
}

/// A fire-and-forget call of the event function, so the caller that
/// produced the event is never blocked on observers. Failures are logged
/// and swallowed.
pub struct IiiEventSink {
    iii: Arc<IIIClient>,
}

impl IiiEventSink {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl EventSink for IiiEventSink {
    fn observed(&self, function_id: &str) -> bool {
        self.iii.is_observed(function_id)
    }

    async fn call(&self, function_id: &str, payload: Value) {
        let request = CallRequest {
            function_id: function_id.to_string(),
            payload,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        if let Err(e) = self.iii.call(request).await {
            tracing::warn!(function_id, error = %e, "event call failed");
        }
    }
}

/// Builds and sends each event, only while its function is observed.
pub struct Emitter {
    sink: Arc<dyn EventSink>,
}

impl Emitter {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self { sink }
    }

    pub async fn emit<T: Serialize>(&self, kind: EventKind, event: &T) {
        let function_id = kind.function_id();
        if !self.sink.observed(function_id) {
            return;
        }
        let payload = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "event failed to serialize");
                return;
            }
        };
        self.sink.call(function_id, payload).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records calls; observes only the function ids it was given.
    struct Recorder {
        observed: Vec<&'static str>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl EventSink for Recorder {
        fn observed(&self, function_id: &str) -> bool {
            self.observed.contains(&function_id)
        }

        async fn call(&self, function_id: &str, payload: Value) {
            self.calls
                .lock()
                .unwrap()
                .push((function_id.to_string(), payload));
        }
    }

    fn stopped(session_id: &str) -> SessionStoppedEvent {
        SessionStoppedEvent {
            session_id: session_id.to_string(),
            reason: "stopped".to_string(),
            timestamp: 0,
        }
    }

    #[test]
    fn every_kind_names_its_function() {
        let ids: Vec<&str> = EventKind::all()
            .iter()
            .map(EventKind::function_id)
            .collect();
        assert_eq!(
            ids,
            vec![ON_SESSION_STARTED_FN, ON_SESSION_STOPPED_FN, ON_ACTIVITY_FN]
        );
        for kind in EventKind::all() {
            assert!(kind.function_id().starts_with("computer::on-"));
            assert!(kind.description().contains("input.session_id"));
        }
    }

    #[tokio::test]
    async fn emit_calls_the_function_only_while_it_is_observed() {
        let recorder = Arc::new(Recorder {
            observed: vec![ON_SESSION_STOPPED_FN],
            calls: Mutex::new(Vec::new()),
        });
        let emitter = Emitter::new(recorder.clone());
        emitter
            .emit(EventKind::SessionStopped, &stopped("c1"))
            .await;
        emitter
            .emit(
                EventKind::Activity,
                &ActivityEvent {
                    session_id: "c1".to_string(),
                    function: "computer::act".to_string(),
                    timestamp: 0,
                },
            )
            .await;

        let calls = recorder.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "computer::on-activity is not observed");
        let (function_id, payload) = &calls[0];
        assert_eq!(function_id, ON_SESSION_STOPPED_FN);
        assert_eq!(payload["session_id"], "c1");
        assert_eq!(payload["reason"], "stopped");
    }
}
