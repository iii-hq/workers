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
        // Defend against re-registration with the same instance_id: drop any
        // prior Subscriber row under the old key before pushing the new one,
        // otherwise a single trigger would fire twice (or N times after N
        // re-registers).
        if let Some((_, old_key)) = self.by_id.remove(&instance_id) {
            let mut purge_old_slot = false;
            if let Some(mut entry) = self.subs.get_mut(&old_key) {
                entry.retain(|s| s.instance_id != instance_id);
                purge_old_slot = entry.is_empty();
            }
            if purge_old_slot {
                self.subs.remove(&old_key);
            }
        }

        let key = (account, folder);
        self.subs.entry(key.clone()).or_default().push(Subscriber {
            instance_id: instance_id.clone(),
            function_id,
            handler_timeout_ms,
        });
        self.by_id.insert(instance_id, key);
    }

    pub fn unregister(&self, instance_id: &str) {
        if let Some((_, key)) = self.by_id.remove(instance_id) {
            if let Some(mut entry) = self.subs.get_mut(&key) {
                entry.retain(|s| s.instance_id != instance_id);
            }
        }
    }

    pub fn subscribers_for(&self, account: &str, folder: &str) -> Vec<Subscriber> {
        let key = (account.to_string(), folder.to_string());
        self.subs.get(&key).map(|e| e.clone()).unwrap_or_default()
    }
}

pub type SharedRegistry = Arc<TriggerRegistry>;
