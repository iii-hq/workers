//! `shell::turns::changed` — the session change history moved.
//!
//! The IDE's Timeline lists a session's turns with `shell::turns::list`. It
//! used to ask again every 1.5 s while a turn ran, since the record grows
//! with every write the turn makes. Now the turn log says so instead: every
//! time a session's record is stored, bindings on that session are woken
//! with `{ session_id }`, at most once per `COALESCE_MS` per session (an
//! agent writing four hundred files is one wake per window, not four
//! hundred), and the subscriber reads the list once.
//!
//! Delivery mirrors [`crate::job_events`]: `Void` routing, each fire on its
//! own task, the binding's `metadata` and `namespace` passed through.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The trigger type a surface binds to follow a session's change history.
pub const TURNS_CHANGED: &str = "shell::turns::changed";

/// Folds a burst of record writes into one wake.
const COALESCE_MS: u64 = 200;

/// Bind with `{ session_id }` for one session; omit it for every session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TurnsChangedConfig {
    /// The top-level session to follow. A sub-agent's changes are recorded
    /// under the session that spawned it, so they wake this binding too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// What changed: a session's record. Ask `shell::turns::list` for it.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct TurnsChangedEvent {
    pub session_id: String,
}

#[derive(Debug, Clone)]
struct Subscriber {
    function_id: String,
    metadata: Option<Value>,
    namespace: Option<String>,
    session_id: Option<String>,
}

impl Subscriber {
    fn from_config(config: &TriggerConfig) -> Result<Self, Error> {
        let filter: TurnsChangedConfig = serde_json::from_value(config.config.clone())
            .map_err(|e| Error::Handler(format!("invalid {TURNS_CHANGED} config: {e}")))?;
        Ok(Self {
            function_id: config.function_id.clone(),
            metadata: config.metadata.clone(),
            namespace: config.namespace.clone(),
            session_id: filter.session_id,
        })
    }

    fn wants(&self, session_id: &str) -> bool {
        self.session_id.as_deref().is_none_or(|id| id == session_id)
    }

    fn request(&self, payload: Value) -> iii_sdk::protocol::TriggerRequestWithMetadata {
        let mut request: iii_sdk::protocol::TriggerRequestWithMetadata = TriggerRequest {
            function_id: self.function_id.clone(),
            payload,
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        }
        .into();
        if let Some(metadata) = &self.metadata {
            request = request.metadata(metadata.clone());
        }
        if let Some(namespace) = &self.namespace {
            request = request.namespace(namespace.clone());
        }
        request
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The bindings, and the sessions with a wake already scheduled.
pub struct TurnsChanged {
    /// `None` in unit tests: fires are recorded, not sent.
    iii: Option<IIIClient>,
    subscribers: Mutex<HashMap<String, Subscriber>>,
    pending: Mutex<HashSet<String>>,
    #[cfg(test)]
    fired: Mutex<Vec<(String, TurnsChangedEvent)>>,
}

impl TurnsChanged {
    pub fn new(iii: Option<IIIClient>) -> Arc<Self> {
        Arc::new(Self {
            iii,
            subscribers: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashSet::new()),
            #[cfg(test)]
            fired: Mutex::new(Vec::new()),
        })
    }

    /// The record of `session_id` was stored. Schedules one wake for the
    /// session unless one is already due; never blocks the writer.
    pub fn notify(self: &Arc<Self>, session_id: &str) {
        if !lock(&self.subscribers)
            .values()
            .any(|s| s.wants(session_id))
        {
            return;
        }
        if !lock(&self.pending).insert(session_id.to_string()) {
            return;
        }
        let this = self.clone();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(COALESCE_MS)).await;
            // Cleared before the read: a write landing meanwhile schedules
            // the next wake, so none is lost.
            lock(&this.pending).remove(&session_id);
            this.fire(&session_id);
        });
    }

    fn fire(&self, session_id: &str) {
        let targets: Vec<Subscriber> = lock(&self.subscribers)
            .values()
            .filter(|s| s.wants(session_id))
            .cloned()
            .collect();
        let event = TurnsChangedEvent {
            session_id: session_id.to_string(),
        };
        #[cfg(test)]
        for target in &targets {
            lock(&self.fired).push((target.function_id.clone(), event.clone()));
        }
        let Some(iii) = &self.iii else {
            return;
        };
        let Ok(payload) = serde_json::to_value(&event) else {
            return;
        };
        for target in targets {
            let iii = iii.clone();
            let request = target.request(payload.clone());
            let function_id = target.function_id;
            tokio::spawn(async move {
                if let Err(e) = iii.trigger(request).await {
                    tracing::warn!(function_id = %function_id, error = %e, "{TURNS_CHANGED} fan-out failed");
                }
            });
        }
    }
}

#[cfg(test)]
impl TurnsChanged {
    /// Bind `probe::<id>` as the trigger handler would.
    pub(crate) fn bind_for_test(&self, id: &str, session_id: Option<&str>) {
        lock(&self.subscribers).insert(
            id.to_string(),
            Subscriber {
                function_id: format!("probe::{id}"),
                metadata: None,
                namespace: None,
                session_id: session_id.map(str::to_string),
            },
        );
    }

    /// The sessions woken so far, in order.
    pub(crate) fn fired_sessions(&self) -> Vec<String> {
        lock(&self.fired)
            .iter()
            .map(|(_, event)| event.session_id.clone())
            .collect()
    }
}

struct TurnsChangedTriggerHandler(Arc<TurnsChanged>);

#[async_trait]
impl TriggerHandler for TurnsChangedTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let subscriber = Subscriber::from_config(&config)?;
        tracing::info!(
            trigger_type = TURNS_CHANGED,
            id = %config.id,
            session_id = ?subscriber.session_id,
            "turn history watch registered"
        );
        lock(&self.0.subscribers).insert(config.id, subscriber);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        lock(&self.0.subscribers).remove(&config.id);
        Ok(())
    }
}

/// Register the trigger type; the returned feed is what the turn log
/// notifies on every stored record.
pub fn register_turns_changed_trigger(iii: &IIIClient) -> Arc<TurnsChanged> {
    let feed = TurnsChanged::new(Some(iii.clone()));
    let _handle = iii.register_trigger_type(
        RegisterTriggerType::new(
            TURNS_CHANGED,
            "Fires when a session's change history (shell::turns::list) changes: a turn \
             opened or closed, a file change recorded, a snapshot taken. Bind with config: \
             { session_id } for one top-level session, or omit it for every session. At \
             most one event per session per ~200 ms, carrying { session_id }; read the \
             turns with shell::turns::list.",
            TurnsChangedTriggerHandler(feed.clone()),
        )
        .trigger_request_format::<TurnsChangedConfig>()
        .call_request_format::<TurnsChangedEvent>(),
    );
    feed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn binding(id: &str, session_id: Option<&str>) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: format!("probe::{id}"),
            config: session_id.map_or(json!({}), |s| json!({ "session_id": s })),
            metadata: Some(json!({ "__binding": id })),
            namespace: None,
        }
    }

    fn fired(feed: &TurnsChanged) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = lock(&feed.fired)
            .iter()
            .map(|(f, e)| (f.clone(), e.session_id.clone()))
            .collect();
        out.sort();
        out
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_writes_is_one_wake_per_session_and_binding() {
        let feed = TurnsChanged::new(None);
        let handler = TurnsChangedTriggerHandler(feed.clone());
        handler
            .register_trigger(binding("mine", Some("s1")))
            .await
            .unwrap();
        handler
            .register_trigger(binding("all", None))
            .await
            .unwrap();
        for _ in 0..50 {
            feed.notify("s1");
        }
        feed.notify("s2");
        tokio::time::sleep(Duration::from_millis(COALESCE_MS + 50)).await;
        assert_eq!(
            fired(&feed),
            [
                ("probe::all".to_string(), "s1".to_string()),
                ("probe::all".to_string(), "s2".to_string()),
                ("probe::mine".to_string(), "s1".to_string()),
            ]
        );
        // The next write after the window is a new wake.
        feed.notify("s1");
        tokio::time::sleep(Duration::from_millis(COALESCE_MS + 50)).await;
        assert_eq!(fired(&feed).len(), 5);
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_scheduled_without_a_binding_and_unbinding_stops_wakes() {
        let feed = TurnsChanged::new(None);
        feed.notify("s1");
        assert!(lock(&feed.pending).is_empty());
        let handler = TurnsChangedTriggerHandler(feed.clone());
        handler
            .register_trigger(binding("b", Some("s1")))
            .await
            .unwrap();
        feed.notify("s2");
        assert!(lock(&feed.pending).is_empty(), "another session's write");
        handler
            .unregister_trigger(binding("b", Some("s1")))
            .await
            .unwrap();
        feed.notify("s1");
        tokio::time::sleep(Duration::from_millis(COALESCE_MS + 50)).await;
        assert!(fired(&feed).is_empty());
    }

    #[tokio::test]
    async fn a_bad_config_is_refused() {
        let handler = TurnsChangedTriggerHandler(TurnsChanged::new(None));
        let err = handler
            .register_trigger(TriggerConfig {
                config: json!({ "session_id": 7 }),
                ..binding("b", None)
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid"), "{err}");
    }
}
