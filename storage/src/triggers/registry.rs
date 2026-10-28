//! Per-(bucket, event-kind) function-id registry. Pollers run per provider
//! source; each poller looks up the registry to find which handler functions
//! should receive each event.
//!
//! Why static (always-on) pollers + dynamic registration: the alternative —
//! spawn one poller per `register_trigger` call — would either require
//! exclusive ownership of the upstream queue (so two registrations on the
//! same bucket couldn't share a poller without splitting messages between
//! them) or a fan-out broadcaster bolted on top. Easier to invert: poll the
//! source once at the worker level, then fan out per registered function.

use crate::triggers::normalize::EventKind;
use async_trait::async_trait;
use iii_sdk::errors::Error;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Bindings (trigger type) and observers (observable function) on one
/// (bucket, kind) receive the same events from one fan-out.
#[derive(Default)]
pub struct TriggerRegistry {
    /// Map (worker-bucket, kind) → set of (instance_id, function_id) pairs.
    /// instance_id is the engine-assigned trigger registration id; we keep it
    /// so `unregister_trigger` can pop the right entry.
    inner: Mutex<HashMap<(String, EventKind), Vec<RegisteredSubscriber>>>,
    /// Map (worker-bucket, kind) → observers of `storage::on-object-created`
    /// / `storage::on-object-deleted`, keyed by observer id.
    observers: Mutex<HashMap<(String, EventKind), Vec<RegisteredObserver>>>,
}

/// One observer of an `storage::on-object-*` function as a sink of the
/// registry: the SDK's `Observer` in production, a recorder in tests.
#[async_trait]
pub trait ObjectObserver: Send + Sync + 'static {
    fn id(&self) -> &str;
    /// Hands one event payload to the observer's target (`obs.call`) and
    /// returns the target's result.
    async fn deliver(&self, payload: &Value, timeout: Duration) -> Result<Value, Error>;
}

#[derive(Clone)]
pub struct RegisteredObserver {
    pub observer: Arc<dyn ObjectObserver>,
    pub handler_timeout_ms: u64,
}

#[derive(Clone, Debug)]
pub struct RegisteredSubscriber {
    pub instance_id: String,
    pub function_id: String,
    pub handler_timeout_ms: u64,
}

impl TriggerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        bucket: String,
        kind: EventKind,
        instance_id: String,
        function_id: String,
        handler_timeout_ms: u64,
    ) {
        let mut g = self.inner.lock().unwrap();
        let entry = g.entry((bucket, kind)).or_default();
        // Idempotent — same instance_id twice replaces the prior subscriber.
        entry.retain(|s| s.instance_id != instance_id);
        entry.push(RegisteredSubscriber {
            instance_id,
            function_id,
            handler_timeout_ms,
        });
    }

    pub fn unregister(&self, instance_id: &str) {
        let mut g = self.inner.lock().unwrap();
        for v in g.values_mut() {
            v.retain(|s| s.instance_id != instance_id);
        }
        // Drop empty entries so subscribers_for returns an empty Vec, not None.
        g.retain(|_, v| !v.is_empty());
    }

    pub fn subscribers_for(&self, bucket: &str, kind: EventKind) -> Vec<RegisteredSubscriber> {
        self.inner
            .lock()
            .unwrap()
            .get(&(bucket.to_string(), kind))
            .cloned()
            .unwrap_or_default()
    }

    /// Files an observer under (bucket, kind); the same id again replaces it.
    pub fn add_observer(
        &self,
        bucket: String,
        kind: EventKind,
        observer: Arc<dyn ObjectObserver>,
        handler_timeout_ms: u64,
    ) {
        let id = observer.id().to_owned();
        let mut g = self.observers.lock().unwrap();
        for v in g.values_mut() {
            v.retain(|o| o.observer.id() != id);
        }
        g.retain(|_, v| !v.is_empty());
        g.entry((bucket, kind))
            .or_default()
            .push(RegisteredObserver {
                observer,
                handler_timeout_ms,
            });
    }

    pub fn remove_observer(&self, id: &str) {
        let mut g = self.observers.lock().unwrap();
        for v in g.values_mut() {
            v.retain(|o| o.observer.id() != id);
        }
        g.retain(|_, v| !v.is_empty());
    }

    pub fn observers_for(&self, bucket: &str, kind: EventKind) -> Vec<RegisteredObserver> {
        self.observers
            .lock()
            .unwrap()
            .get(&(bucket.to_string(), kind))
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_lookup() {
        let r = TriggerRegistry::new();
        r.register(
            "uploads".into(),
            EventKind::Created,
            "i1".into(),
            "f1".into(),
            60_000,
        );
        let subs = r.subscribers_for("uploads", EventKind::Created);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].function_id, "f1");
        assert!(r.subscribers_for("uploads", EventKind::Deleted).is_empty());
        assert!(r.subscribers_for("other", EventKind::Created).is_empty());
    }

    #[test]
    fn unregister_removes_only_matching_instance() {
        let r = TriggerRegistry::new();
        r.register(
            "uploads".into(),
            EventKind::Created,
            "i1".into(),
            "f1".into(),
            60_000,
        );
        r.register(
            "uploads".into(),
            EventKind::Created,
            "i2".into(),
            "f2".into(),
            60_000,
        );
        r.unregister("i1");
        let subs = r.subscribers_for("uploads", EventKind::Created);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].function_id, "f2");
    }

    #[test]
    fn re_registration_with_same_instance_replaces() {
        let r = TriggerRegistry::new();
        r.register(
            "uploads".into(),
            EventKind::Created,
            "i1".into(),
            "f1".into(),
            60_000,
        );
        r.register(
            "uploads".into(),
            EventKind::Created,
            "i1".into(),
            "f1-replaced".into(),
            30_000,
        );
        let subs = r.subscribers_for("uploads", EventKind::Created);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].function_id, "f1-replaced");
        assert_eq!(subs[0].handler_timeout_ms, 30_000);
    }
}
