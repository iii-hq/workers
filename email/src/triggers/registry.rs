//! (account, folder) -> Vec<Subscriber> mapping. Ported from
//! `storage::triggers::registry`. Subscriber entries are keyed by trigger
//! instance id so unregister pops the right row.

use dashmap::DashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Subscriber {
    pub instance_id: String,
    pub function_id: String,
    pub handler_timeout_ms: u64,
}

#[derive(Default)]
pub struct TriggerRegistry {
    subs: DashMap<(String, String), Vec<Subscriber>>,
    by_id: DashMap<String, (String, String)>,
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
        handler_timeout_ms: u64,
    ) {
        // Drop any prior row with this instance_id so a re-register replaces
        // rather than duplicates the subscriber.
        self.unregister(&instance_id);

        let key = (account, folder);
        self.subs.entry(key.clone()).or_default().push(Subscriber {
            instance_id: instance_id.clone(),
            function_id,
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
}

pub type SharedRegistry = Arc<TriggerRegistry>;
