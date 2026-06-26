//! In-memory ephemeral subscription registry (harness.md § Subscriptions).
//!
//! Subscriptions are intentionally NOT persisted: they live for the harness
//! process only (the "ephemeral" contract). Keyed by subscription id and indexed
//! by owning session for the per-session cap and session-deleted cleanup.
//! Lifecycle is covered by explicit `unsubscribe`, `once` self-teardown,
//! `session::deleted`, and the engine's worker-disconnect GC on process exit —
//! so there is no TTL/sweep here.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use iii_sdk::{FunctionRef, Trigger};

/// One live subscription: its descriptor, the iii handles to tear it down, and a
/// fire counter. Held in an `Arc` so a firing handler reads it without holding
/// the registry lock.
pub struct SubEntry {
    pub id: String,
    /// Owning session — closure-captured at registration; the agent can never
    /// widen the target (it is injected by the trusted dispatch layer).
    pub session_id: String,
    pub trigger_type: String,
    pub label: Option<String>,
    pub once: bool,
    /// Monotonic fire count, used only for an idempotent notification entry id.
    pub fire_count: AtomicU64,
    /// The per-sub internal handler; unregistered on teardown.
    pub function: FunctionRef,
    /// The iii trigger binding; set just after registration, taken on teardown.
    pub trigger: Mutex<Option<Trigger>>,
}

#[derive(Default)]
struct Inner {
    by_id: HashMap<String, Arc<SubEntry>>,
    by_session: HashMap<String, HashSet<String>>,
}

/// The process-wide registry of active ephemeral subscriptions.
pub struct SubscriptionRegistry {
    inner: Mutex<Inner>,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Number of active subscriptions owned by a session (cap check).
    pub fn count_for(&self, session_id: &str) -> usize {
        self.lock()
            .by_session
            .get(session_id)
            .map(HashSet::len)
            .unwrap_or(0)
    }

    /// Add an entry. Inserted BEFORE the trigger is bound so an immediate fire
    /// still finds it (see [`set_trigger`](Self::set_trigger)).
    pub fn insert(&self, entry: Arc<SubEntry>) {
        let mut inner = self.lock();
        inner
            .by_session
            .entry(entry.session_id.clone())
            .or_default()
            .insert(entry.id.clone());
        inner.by_id.insert(entry.id.clone(), entry);
    }

    pub fn get(&self, sub_id: &str) -> Option<Arc<SubEntry>> {
        self.lock().by_id.get(sub_id).cloned()
    }

    /// Attach the trigger handle after a successful bind. If the entry was torn
    /// down in the meantime, unregister the trigger immediately to avoid an
    /// orphaned engine binding.
    pub fn set_trigger(&self, sub_id: &str, trigger: Trigger) {
        match self.lock().by_id.get(sub_id) {
            Some(entry) => {
                *entry.trigger.lock().unwrap_or_else(|p| p.into_inner()) = Some(trigger);
            }
            None => trigger.unregister(),
        }
    }

    /// Record one fire: bump the counter and report `(fire_count, once)`. Returns
    /// `None` only when the subscription is already gone.
    pub fn record_fire(&self, sub_id: &str) -> Option<(u64, bool)> {
        let entry = self.get(sub_id)?;
        let fire_count = entry.fire_count.fetch_add(1, Ordering::SeqCst) + 1;
        Some((fire_count, entry.once))
    }

    /// Remove a subscription, unregistering its trigger THEN its function (stop
    /// new fires before dropping the handler). Returns whether it existed.
    pub fn remove(&self, sub_id: &str) -> bool {
        let entry = {
            let mut inner = self.lock();
            let Some(entry) = inner.by_id.remove(sub_id) else {
                return false;
            };
            if let Some(set) = inner.by_session.get_mut(&entry.session_id) {
                set.remove(sub_id);
                if set.is_empty() {
                    inner.by_session.remove(&entry.session_id);
                }
            }
            entry
        };
        // Tear down outside the lock (the SDK calls send messages).
        if let Some(trigger) = entry
            .trigger
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            trigger.unregister();
        }
        entry.function.unregister();
        true
    }

    /// Remove every subscription owned by a session (session-deleted cleanup).
    pub fn remove_session(&self, session_id: &str) -> usize {
        let ids: Vec<String> = {
            let inner = self.lock();
            inner
                .by_session
                .get(session_id)
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default()
        };
        ids.iter().filter(|id| self.remove(id)).count()
    }
}

impl Default for SubscriptionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
