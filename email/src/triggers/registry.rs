//! (account, folder) -> Vec<Subscriber> mapping. Ported from
//! `storage::triggers::registry`. Subscriber entries are keyed by trigger
//! instance id so unregister pops the right row.
//!
//! The same registry files the observers of `email::on-new-mail` (iii 0.25)
//! under the (account, folder) of their args, keyed by observer id, so one
//! IDLE event reaches `email::new-mail` bindings and observers alike.

use async_trait::async_trait;
use dashmap::DashMap;
use iii_sdk::errors::Error;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Subscriber {
    pub instance_id: String,
    pub function_id: String,
    pub namespace: Option<String>,
    pub handler_timeout_ms: u64,
}

/// One observer of `email::on-new-mail`, as the dispatcher delivers to it.
/// [`iii_sdk::Observer`] delivers with `obs.call`.
#[async_trait]
pub trait NewMailObserver: Send + Sync {
    fn id(&self) -> &str;
    /// Hand one event payload to the observer's target.
    async fn deliver(&self, payload: Value, timeout: Duration) -> Result<(), Error>;
}

/// An observer filed under an (account, folder).
#[derive(Clone)]
pub struct ObserverSub {
    pub observer: Arc<dyn NewMailObserver>,
    pub handler_timeout_ms: u64,
}

#[derive(Default)]
pub struct TriggerRegistry {
    subs: DashMap<(String, String), Vec<Subscriber>>,
    by_id: DashMap<String, (String, String)>,
    observers: DashMap<(String, String), Vec<ObserverSub>>,
    observer_by_id: DashMap<String, (String, String)>,
}

impl TriggerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        account: String,
        folder: String,
        instance_id: String,
        function_id: String,
        namespace: Option<String>,
        handler_timeout_ms: u64,
    ) {
        // Drop any prior row with this instance_id so a re-register replaces
        // rather than duplicates the subscriber.
        self.unregister(&instance_id);

        let key = (account, folder);
        self.subs.entry(key.clone()).or_default().push(Subscriber {
            instance_id: instance_id.clone(),
            function_id,
            namespace,
            handler_timeout_ms,
        });
        self.by_id.insert(instance_id, key);
    }

    pub fn unregister(&self, instance_id: &str) {
        let Some((_, key)) = self.by_id.remove(instance_id) else {
            return;
        };
        let purge = if let Some(mut entry) = self.subs.get_mut(&key) {
            entry.retain(|s| s.instance_id != instance_id);
            entry.is_empty()
        } else {
            false
        };
        if purge {
            self.subs.remove(&key);
        }
    }

    pub fn subscribers_for(&self, account: &str, folder: &str) -> Vec<Subscriber> {
        let key = (account.to_string(), folder.to_string());
        self.subs.get(&key).map(|e| e.clone()).unwrap_or_default()
    }

    /// File an observer of `email::on-new-mail` under (account, folder). A
    /// second add with the same observer id replaces the first.
    pub fn add_observer(
        &self,
        account: String,
        folder: String,
        observer: Arc<dyn NewMailObserver>,
        handler_timeout_ms: u64,
    ) {
        let id = observer.id().to_string();
        self.remove_observer(&id);

        let key = (account, folder);
        self.observers
            .entry(key.clone())
            .or_default()
            .push(ObserverSub {
                observer,
                handler_timeout_ms,
            });
        self.observer_by_id.insert(id, key);
    }

    /// Remove an observer; true when it was filed.
    pub fn remove_observer(&self, observer_id: &str) -> bool {
        let Some((_, key)) = self.observer_by_id.remove(observer_id) else {
            return false;
        };
        let purge = if let Some(mut entry) = self.observers.get_mut(&key) {
            entry.retain(|o| o.observer.id() != observer_id);
            entry.is_empty()
        } else {
            false
        };
        if purge {
            self.observers.remove(&key);
        }
        true
    }

    pub fn observers_for(&self, account: &str, folder: &str) -> Vec<ObserverSub> {
        let key = (account.to_string(), folder.to_string());
        self.observers
            .get(&key)
            .map(|e| e.clone())
            .unwrap_or_default()
    }

    pub fn observer_count(&self) -> usize {
        self.observer_by_id.len()
    }
}

pub type SharedRegistry = Arc<TriggerRegistry>;
