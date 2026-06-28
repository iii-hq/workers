//! In-memory ephemeral subscription index (harness.md § Subscriptions).
//!
//! The engine owns each subscription's trigger AND its delivery proxy; the
//! harness keeps only the small bookkeeping it needs LOCALLY: the owning session
//! (per-session cap, ownership check, session-deleted cleanup), a fire counter
//! (for idempotent notification entry ids), and the engine-returned trigger id
//! (to unregister). No iii handles are held here.
//!
//! Subscriptions are intentionally NOT persisted: they live for the harness
//! process only (the "ephemeral" contract). Lifecycle is covered by explicit
//! `unsubscribe`, `once` self-teardown, `session::deleted`, and the engine's
//! worker-disconnect GC on process exit — so there is no TTL/sweep here.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// One live subscription's local bookkeeping. Held in an `Arc` so a firing
/// handler reads it without holding the registry lock.
pub struct SubEntry {
    /// Owning session — captured at registration from the trusted dispatch
    /// layer; the agent can never widen the target.
    pub session_id: String,
    pub label: Option<String>,
    once: bool,
    /// Monotonic fire count, used only for an idempotent notification entry id.
    seq: AtomicU64,
    /// The engine-returned trigger id; attached just after registration (see
    /// [`set_trigger_id`](SubscriptionRegistry::set_trigger_id)) and used to
    /// `engine::unregister_trigger` on teardown.
    trigger_id: Mutex<Option<String>>,
}

impl SubEntry {
    fn new(session_id: String, once: bool, label: Option<String>) -> Self {
        Self {
            session_id,
            label,
            once,
            seq: AtomicU64::new(0),
            trigger_id: Mutex::new(None),
        }
    }
}

/// Returned by [`SubscriptionRegistry::try_insert`] when the per-session cap is hit.
#[derive(Debug)]
pub struct CapExceeded;

/// A claimed fire, resolved against the local registry authority.
pub struct ClaimedFire {
    pub session_id: String,
    pub label: Option<String>,
    pub trigger_id: Option<String>,
    pub sequence: Option<u64>,
}

impl ClaimedFire {
    pub fn entry_id(&self, sub_id: &str) -> String {
        match self.sequence {
            Some(seq) => format!("e_notify_{sub_id}_{seq}"),
            None => format!("e_notify_{sub_id}"),
        }
    }
}

#[derive(Default)]
struct Inner {
    by_id: HashMap<String, Arc<SubEntry>>,
    by_session: HashMap<String, HashSet<String>>,
}

/// The process-wide index of active ephemeral subscriptions.
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

    /// Atomically enforce the per-session cap and insert a placeholder entry
    /// (its trigger id is attached later via [`set_trigger_id`](Self::set_trigger_id)).
    /// Inserting BEFORE the engine bind means an immediate fire still finds it.
    pub fn try_insert(
        &self,
        sub_id: &str,
        session_id: &str,
        once: bool,
        label: Option<String>,
        max: usize,
    ) -> Result<(), CapExceeded> {
        let mut inner = self.lock();
        let count = inner
            .by_session
            .get(session_id)
            .map(HashSet::len)
            .unwrap_or(0);
        if count >= max {
            return Err(CapExceeded);
        }
        inner
            .by_session
            .entry(session_id.to_string())
            .or_default()
            .insert(sub_id.to_string());
        inner.by_id.insert(
            sub_id.to_string(),
            Arc::new(SubEntry::new(session_id.to_string(), once, label)),
        );
        Ok(())
    }

    /// Attach the engine-returned trigger id after a successful bind. Returns
    /// `false` if the entry is already gone (a `once` fire won the race in the
    /// bind window) — the caller must then unregister the orphan trigger.
    pub fn set_trigger_id(&self, sub_id: &str, trigger_id: &str) -> bool {
        match self.lock().by_id.get(sub_id) {
            Some(entry) => {
                *entry.trigger_id.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some(trigger_id.to_string());
                true
            }
            None => false,
        }
    }

    /// Claim a fired subscription using local registry authority. One-shot
    /// subscriptions are removed; recurring subscriptions keep their entry and
    /// receive a monotonic sequence.
    pub fn claim_fire(&self, sub_id: &str) -> Option<ClaimedFire> {
        let mut inner = self.lock();
        let entry = inner.by_id.get(sub_id).cloned()?;
        if entry.once {
            let entry = remove_entry(&mut inner, sub_id)?;
            return Some(ClaimedFire {
                session_id: entry.session_id.clone(),
                label: entry.label.clone(),
                trigger_id: trigger_id(&entry),
                sequence: None,
            });
        }
        Some(ClaimedFire {
            session_id: entry.session_id.clone(),
            label: entry.label.clone(),
            trigger_id: None,
            sequence: Some(entry.seq.fetch_add(1, Ordering::SeqCst) + 1),
        })
    }

    /// The owning session of a subscription (unsubscribe ownership check).
    pub fn session_of(&self, sub_id: &str) -> Option<String> {
        self.lock().by_id.get(sub_id).map(|e| e.session_id.clone())
    }

    /// Atomically remove and return `(session_id, trigger_id)`. Winner-takes-all:
    /// only the first caller for a given id gets `Some` — this is the `once` /
    /// unsubscribe claim that guarantees at-most-once delivery + teardown.
    pub fn take(&self, sub_id: &str) -> Option<(String, Option<String>)> {
        let mut inner = self.lock();
        let entry = remove_entry(&mut inner, sub_id)?;
        Some((entry.session_id.clone(), trigger_id(&entry)))
    }

    /// Remove every subscription owned by a session and return each
    /// `(sub_id, trigger_id)` so the caller can `engine::unregister_trigger` them.
    pub fn take_session(&self, session_id: &str) -> Vec<(String, Option<String>)> {
        let ids: Vec<String> = {
            let inner = self.lock();
            inner
                .by_session
                .get(session_id)
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default()
        };
        ids.into_iter()
            .filter_map(|id| {
                self.take(&id)
                    .map(|(_session, trigger_id)| (id, trigger_id))
            })
            .collect()
    }
}

fn trigger_id(entry: &SubEntry) -> Option<String> {
    entry
        .trigger_id
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

fn remove_entry(inner: &mut Inner, sub_id: &str) -> Option<Arc<SubEntry>> {
    let entry = inner.by_id.remove(sub_id)?;
    if let Some(set) = inner.by_session.get_mut(&entry.session_id) {
        set.remove(sub_id);
        if set.is_empty() {
            inner.by_session.remove(&entry.session_id);
        }
    }
    Some(entry)
}

impl Default for SubscriptionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_insert_enforces_cap_atomically() {
        let reg = SubscriptionRegistry::new();
        assert!(reg.try_insert("sub_1", "s", false, None, 2).is_ok());
        assert!(reg.try_insert("sub_2", "s", false, None, 2).is_ok());
        assert!(reg.try_insert("sub_3", "s", false, None, 2).is_err());
        // A different session has its own budget.
        assert!(reg.try_insert("sub_4", "s2", false, None, 2).is_ok());
    }

    #[test]
    fn set_trigger_id_then_take_returns_it() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "s", false, None, 8).unwrap();
        assert!(reg.set_trigger_id("sub_1", "trig_1"));
        let (session, trigger_id) = reg.take("sub_1").expect("entry present");
        assert_eq!(session, "s");
        assert_eq!(trigger_id.as_deref(), Some("trig_1"));
    }

    #[test]
    fn set_trigger_id_false_after_take() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "s", false, None, 8).unwrap();
        assert!(reg.take("sub_1").is_some());
        // Entry gone (a once-fire won the window): caller must unregister the orphan.
        assert!(!reg.set_trigger_id("sub_1", "trig_1"));
    }

    #[test]
    fn take_is_idempotent_winner_takes_all() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "s", false, None, 8).unwrap();
        assert!(reg.take("sub_1").is_some());
        assert!(reg.take("sub_1").is_none());
    }

    #[test]
    fn session_of_and_take_session() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "s", false, None, 8).unwrap();
        reg.try_insert("sub_2", "s", false, None, 8).unwrap();
        reg.set_trigger_id("sub_1", "trig_1");
        assert_eq!(reg.session_of("sub_1").as_deref(), Some("s"));
        let mut dropped = reg.take_session("s");
        dropped.sort();
        assert_eq!(dropped.len(), 2);
        assert!(reg.session_of("sub_1").is_none());
        assert!(reg.take_session("s").is_empty());
    }

    #[test]
    fn claim_once_fire_uses_registry_owner_and_removes_entry() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "owner", true, Some("done".to_string()), 8)
            .unwrap();
        reg.set_trigger_id("sub_1", "trig_1");

        let claim = reg.claim_fire("sub_1").expect("entry present");
        assert_eq!(claim.session_id, "owner");
        assert_eq!(claim.label.as_deref(), Some("done"));
        assert_eq!(claim.trigger_id.as_deref(), Some("trig_1"));
        assert_eq!(claim.sequence, None);
        assert_eq!(claim.entry_id("sub_1"), "e_notify_sub_1");
        assert!(reg.session_of("sub_1").is_none());
        assert!(reg.claim_fire("sub_1").is_none());
    }

    #[test]
    fn claim_recurring_fire_uses_registry_owner_and_keeps_entry() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "owner", false, None, 8).unwrap();

        let first = reg.claim_fire("sub_1").expect("first fire");
        let second = reg.claim_fire("sub_1").expect("second fire");
        assert_eq!(first.session_id, "owner");
        assert_eq!(first.sequence, Some(1));
        assert_eq!(first.entry_id("sub_1"), "e_notify_sub_1_1");
        assert_eq!(second.sequence, Some(2));
        assert_eq!(second.entry_id("sub_1"), "e_notify_sub_1_2");
        assert_eq!(reg.session_of("sub_1").as_deref(), Some("owner"));
    }
}
