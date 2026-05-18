//! In-process notifier that wakes `run::resume` the moment the FSM
//! persists a terminal record. Replaces the 250 ms poll loop the
//! approval-gate resolver used to run.
//!
//! The same process owns both the executor (which signals on terminal
//! save) and `execute_resume` (which waits). Cross-process resume
//! requires switching to a state-bus subscriber instead — see the PR
//! description for the variant we deferred.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct AwaitingApproval {
    inner: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
}

impl AwaitingApproval {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn slot(&self, session_id: &str) -> Arc<Notify> {
        let mut guard = self.inner.lock().expect("AwaitingApproval mutex poisoned");
        guard
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }

    pub fn signal(&self, session_id: &str) {
        let slot = self.slot(session_id);
        slot.notify_waiters();
    }

    pub fn clear(&self, session_id: &str) {
        let mut guard = self.inner.lock().expect("AwaitingApproval mutex poisoned");
        guard.remove(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn signal_after_arming_wakes_waiter() {
        let awaiting = AwaitingApproval::new();
        let slot = awaiting.slot("s1");
        let notified = slot.notified();

        let signaller = {
            let awaiting = awaiting.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                awaiting.signal("s1");
            })
        };

        tokio::time::timeout(Duration::from_secs(1), notified)
            .await
            .expect("waiter should wake when signal fires");
        signaller.await.unwrap();
    }

    #[tokio::test]
    async fn signal_without_waiter_is_noop() {
        let awaiting = AwaitingApproval::new();
        awaiting.signal("nobody-home");
        // Should not panic, should not block.
    }

    #[tokio::test]
    async fn clear_drops_slot_so_next_call_gets_fresh_notify() {
        let awaiting = AwaitingApproval::new();
        let slot_a = awaiting.slot("s1");
        awaiting.clear("s1");
        let slot_b = awaiting.slot("s1");
        // Different Arc identity after clear.
        assert!(!Arc::ptr_eq(&slot_a, &slot_b));
    }

    #[tokio::test]
    async fn slot_returns_same_arc_for_same_session() {
        let awaiting = AwaitingApproval::new();
        let a = awaiting.slot("s1");
        let b = awaiting.slot("s1");
        assert!(Arc::ptr_eq(&a, &b));
    }

    /// Documents the constraint that drives `execute_resume`'s
    /// arm-then-recheck pattern: a `signal` that fires before any
    /// waiter has called `notified()` is lost. Without re-checking
    /// persistence state between arming and awaiting, a parked
    /// session whose signal arrived during the first load would
    /// hang until the 30 s timeout. Don't "simplify" the recheck
    /// away.
    #[tokio::test]
    async fn signal_before_notified_is_lost() {
        let awaiting = AwaitingApproval::new();
        awaiting.signal("s1");
        let slot = awaiting.slot("s1");
        let notified = slot.notified();

        let elapsed = tokio::time::timeout(Duration::from_millis(50), notified).await;
        assert!(elapsed.is_err(), "signal fired before arming must not wake");
    }
}
