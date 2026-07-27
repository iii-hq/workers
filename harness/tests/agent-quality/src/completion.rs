use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::trigger::Trigger;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Notify;

use crate::error::{EvalError, FailureClass, Phase};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompletionEventV1 {
    pub session_id: String,
    pub turn_id: String,
    pub status: String,
    pub terminal: bool,
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl CompletionEventV1 {
    fn successful(&self) -> bool {
        self.terminal && self.status == "completed" && self.result_error.is_none()
    }
}

#[derive(Clone, Default)]
pub struct CompletionInbox {
    events: Arc<Mutex<HashMap<(String, String), CompletionEventV1>>>,
    notify: Arc<Notify>,
}

impl CompletionInbox {
    pub fn push(&self, event: CompletionEventV1) {
        if !event.terminal {
            return;
        }
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry((event.session_id.clone(), event.turn_id.clone()))
            .or_insert(event);
        self.notify.notify_waiters();
    }

    pub async fn wait_terminal(
        &self,
        session_id: &str,
        turn_id: &str,
        timeout: Duration,
    ) -> Result<CompletionEventV1, EvalError> {
        let key = (session_id.to_string(), turn_id.to_string());
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.notify.notified();
            if let Some(event) = self
                .events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&key)
            {
                return Ok(event);
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Err(EvalError::timeout(
                    Phase::Await,
                    format!(
                        "timed out waiting for harness::turn-completed for session {session_id}, turn {turn_id}"
                    ),
                ));
            }
        }
    }
}

pub fn validate_success(event: &CompletionEventV1) -> Result<(), EvalError> {
    if event.successful() {
        return Ok(());
    }
    Err(EvalError::new(
        FailureClass::SubjectError,
        Phase::Await,
        None,
        Some("harness::turn-completed".into()),
        format!(
            "subject turn ended with status {:?}: {}",
            event.status,
            event
                .result_error
                .as_deref()
                .or(event.reason.as_deref())
                .unwrap_or("no reason supplied")
        ),
    ))
}

pub struct CompletionBinding(Option<Trigger>);

impl CompletionBinding {
    pub fn new(trigger: Trigger) -> Self {
        Self(Some(trigger))
    }

    pub fn unregister(mut self) {
        if let Some(trigger) = self.0.take() {
            trigger.unregister();
        }
    }
}

impl Drop for CompletionBinding {
    fn drop(&mut self) {
        if let Some(trigger) = self.0.take() {
            trigger.unregister();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(session: &str, turn: &str, status: &str, terminal: bool) -> CompletionEventV1 {
        CompletionEventV1 {
            session_id: session.into(),
            turn_id: turn.into(),
            status: status.into(),
            terminal,
            timestamp: 1,
            result: None,
            result_error: None,
            reason: None,
        }
    }

    #[tokio::test]
    async fn receives_terminal_event_by_session_and_turn() {
        let inbox = CompletionInbox::default();
        inbox.push(event("s1", "t1", "completed", false));
        inbox.push(event("s1", "t1", "completed", true));
        let received = inbox
            .wait_terminal("s1", "t1", Duration::from_millis(10))
            .await
            .unwrap();
        assert!(received.successful());
    }

    #[tokio::test]
    async fn missing_event_times_out() {
        let error = CompletionInbox::default()
            .wait_terminal("s1", "t1", Duration::from_millis(1))
            .await
            .unwrap_err();
        assert_eq!(error.record.class, FailureClass::Timeout);
    }

    #[test]
    fn failed_and_cancelled_events_are_subject_errors() {
        for status in ["failed", "cancelled"] {
            let error = validate_success(&event("s1", "t1", status, true)).unwrap_err();
            assert_eq!(error.record.class, FailureClass::SubjectError);
        }
    }
}
