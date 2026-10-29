//! `shell::turns::on-changed` — the session change history moved.
//!
//! The IDE's Timeline lists a session's turns with `shell::turns::list`. It
//! used to ask again every 1.5 s while a turn ran, since the record grows
//! with every write the turn makes. The turn log calls this function each
//! time a session's record is stored, with `{ session_id }`, at most once per
//! `COALESCE_MS` per session (an agent writing four hundred files is one
//! call per window, not four hundred), and the consumer reads the list once.
//!
//! The function takes no args: a consumer selects its session with a filter
//! on `input.session_id`. Nothing is scheduled while nobody observes it.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The function a surface observes to follow a session's change history.
pub const ON_TURNS_CHANGED_FN: &str = "shell::turns::on-changed";

/// Folds a burst of record writes into one call.
const COALESCE_MS: u64 = 200;

/// What changed: a session's record. Ask `shell::turns::list` for it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnsChangedEvent {
    /// The top-level session whose record was stored. A sub-agent's changes
    /// are recorded under the session that spawned it.
    pub session_id: String,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The sessions with a call already scheduled.
pub struct TurnsChanged {
    /// `None` in unit tests: calls are recorded, not sent.
    iii: Option<IIIClient>,
    pending: Mutex<HashSet<String>>,
    #[cfg(test)]
    observed: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fired: Mutex<Vec<TurnsChangedEvent>>,
}

impl TurnsChanged {
    pub fn new(iii: Option<IIIClient>) -> Arc<Self> {
        Arc::new(Self {
            iii,
            pending: Mutex::new(HashSet::new()),
            #[cfg(test)]
            observed: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fired: Mutex::new(Vec::new()),
        })
    }

    /// Whether anyone observes the function: one atomic load.
    fn is_observed(&self) -> bool {
        #[cfg(test)]
        if self.iii.is_none() {
            return self.observed.load(std::sync::atomic::Ordering::Relaxed);
        }
        self.iii
            .as_ref()
            .is_some_and(|iii| iii.is_observed(ON_TURNS_CHANGED_FN))
    }

    /// The record of `session_id` was stored. Schedules one call for the
    /// session unless one is already due; never blocks the writer.
    pub fn notify(self: &Arc<Self>, session_id: &str) {
        if !self.is_observed() {
            return;
        }
        if !lock(&self.pending).insert(session_id.to_string()) {
            return;
        }
        let this = self.clone();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(COALESCE_MS)).await;
            // Cleared before the call: a write landing meanwhile schedules
            // the next one, so none is lost.
            lock(&this.pending).remove(&session_id);
            this.fire(&session_id);
        });
    }

    fn fire(&self, session_id: &str) {
        if !self.is_observed() {
            return;
        }
        let event = TurnsChangedEvent {
            session_id: session_id.to_string(),
        };
        #[cfg(test)]
        lock(&self.fired).push(event.clone());
        let Some(iii) = &self.iii else {
            return;
        };
        let Ok(payload) = serde_json::to_value(&event) else {
            return;
        };
        let iii = iii.clone();
        let request = CallRequest {
            function_id: ON_TURNS_CHANGED_FN.to_string(),
            payload,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        tokio::spawn(async move {
            if let Err(e) = iii.call(request).await {
                tracing::warn!(error = %e, "{ON_TURNS_CHANGED_FN} call failed");
            }
        });
    }
}

#[cfg(test)]
impl TurnsChanged {
    /// Stand in for an observer of the function.
    pub(crate) fn observe_for_test(&self) {
        self.observed
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// The sessions called so far, in order.
    pub(crate) fn fired_sessions(&self) -> Vec<String> {
        lock(&self.fired)
            .iter()
            .map(|event| event.session_id.clone())
            .collect()
    }
}

/// Register `shell::turns::on-changed`; the returned feed is what the turn
/// log notifies on every stored record.
pub fn register_turns_changed(iii: &IIIClient) -> Arc<TurnsChanged> {
    iii.register_function(
        ON_TURNS_CHANGED_FN,
        RegisterFunction::new(|event: TurnsChangedEvent| Ok::<_, Error>(event))
            .description(
                "Called when a session's change history (shell::turns::list) changes: a turn \
                 opened or closed, a file change recorded, a snapshot taken. Observers \
                 receive { session_id }, at most once per session per ~200 ms; filter on \
                 input.session_id for one top-level session, and read the turns with \
                 shell::turns::list.",
            )
            .private(),
    );
    TurnsChanged::new(Some(iii.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_writes_is_one_call_per_session() {
        let feed = TurnsChanged::new(None);
        feed.observe_for_test();
        for _ in 0..50 {
            feed.notify("s1");
        }
        feed.notify("s2");
        tokio::time::sleep(Duration::from_millis(COALESCE_MS + 50)).await;
        let mut fired = feed.fired_sessions();
        fired.sort();
        assert_eq!(fired, ["s1", "s2"]);
        // The next write after the window is a new call.
        feed.notify("s1");
        tokio::time::sleep(Duration::from_millis(COALESCE_MS + 50)).await;
        assert_eq!(feed.fired_sessions().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_scheduled_while_nobody_observes() {
        let feed = TurnsChanged::new(None);
        feed.notify("s1");
        assert!(lock(&feed.pending).is_empty());
        tokio::time::sleep(Duration::from_millis(COALESCE_MS + 50)).await;
        assert!(feed.fired_sessions().is_empty());
    }

    #[test]
    fn the_event_is_the_session_id() {
        let schema = serde_json::to_value(schemars::schema_for!(TurnsChangedEvent)).unwrap();
        assert_eq!(schema["required"], serde_json::json!(["session_id"]));
    }
}
