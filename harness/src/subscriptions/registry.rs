//! In-memory ephemeral subscription index (harness.md § Subscriptions).
//!
//! The engine owns each subscription's trigger AND its fire-time metadata
//! delivery; the harness keeps only the small bookkeeping it needs LOCALLY: the
//! owning session (per-session cap, ownership check, session-deleted cleanup),
//! a fire counter (for idempotent notification entry ids), and the
//! engine-returned trigger id (to unregister). Fire context such as label /
//! once is carried by the engine's `Trigger` metadata, not duplicated here. No
//! iii handles are held here.
//!
//! Subscriptions are intentionally NOT persisted: they live for the harness
//! process only (the "ephemeral" contract). Lifecycle is covered by explicit
//! `unsubscribe`, `once` self-teardown, `session::deleted`, and the engine's
//! worker-disconnect GC on process exit — so there is no TTL/sweep here.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

/// One live subscription's local bookkeeping. Held in an `Arc` so a firing
/// handler reads it without holding the registry lock.
pub struct SubEntry {
    /// Owning session — captured at registration from the trusted dispatch
    /// layer; the agent can never widen the target.
    pub session_id: String,
    /// Monotonic fire count, used only for an idempotent notification entry id.
    seq: AtomicU64,
    /// The engine-returned trigger id; attached just after registration (see
    /// [`set_trigger_id`](SubscriptionRegistry::set_trigger_id)) and used to
    /// `engine::unregister_trigger` on teardown.
    trigger_id: Mutex<Option<String>>,
    /// Canonicalized registration request for same-session idempotency
    /// ([`find_duplicate`](SubscriptionRegistry::find_duplicate)); `Null` when
    /// the caller opted out of dedup.
    dedup_key: Value,
    /// A one-shot notify subscription: the owning session expects to be woken
    /// by its firing to continue work. Drives the `terminal` flag on
    /// `harness::turn-completed` — a session with an armed wake completes
    /// non-terminally. Standing watchers and react bindings don't count.
    wake: bool,
}

impl SubEntry {
    fn new(session_id: String, dedup_key: Value, wake: bool) -> Self {
        Self {
            session_id,
            seq: AtomicU64::new(0),
            trigger_id: Mutex::new(None),
            dedup_key,
            wake,
        }
    }
}

/// Returned by [`SubscriptionRegistry::try_insert`] when the per-session cap is hit.
#[derive(Debug)]
pub struct CapExceeded;

/// A liveness/ownership-checked fired subscription.
#[derive(Debug, PartialEq, Eq)]
pub struct FireClaim {
    pub entry_id: String,
    pub trigger_id: Option<String>,
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
        max: usize,
    ) -> Result<(), CapExceeded> {
        self.try_insert_keyed(sub_id, session_id, max, Value::Null, false)
    }

    /// [`try_insert`](Self::try_insert) with a canonicalized request key so a
    /// later identical registration can be answered idempotently via
    /// [`find_duplicate`](Self::find_duplicate). `wake` marks a one-shot
    /// notify the owning session is waiting on (see [`SubEntry::wake`]).
    pub fn try_insert_keyed(
        &self,
        sub_id: &str,
        session_id: &str,
        max: usize,
        dedup_key: Value,
        wake: bool,
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
            Arc::new(SubEntry::new(session_id.to_string(), dedup_key, wake)),
        );
        Ok(())
    }

    /// Whether the session owns an armed wake (a live one-shot notify
    /// subscription). `false` means nothing will wake this session to continue
    /// — its next completion is terminal. Notify delivery requires a live
    /// registry entry (`claim_fire` drops unknown ids), so this is the truth
    /// about whether a wake can still arrive, restarts included.
    pub fn session_expects_wake(&self, session_id: &str) -> bool {
        let inner = self.lock();
        inner.by_session.get(session_id).is_some_and(|subs| {
            subs.iter()
                .any(|id| inner.by_id.get(id).is_some_and(|e| e.wake))
        })
    }

    /// The standing subscription in `session_id` registered with an identical
    /// request, if any — registration idempotency: a retried or double-issued
    /// `engine::register_trigger` returns the existing subscription instead of
    /// wiring a twin binding. Only fully-bound entries count (trigger id
    /// attached); an in-flight parallel registration is left to race. `Null`
    /// keys (unkeyed inserts) never match.
    pub fn find_duplicate(&self, session_id: &str, key: &Value) -> Option<String> {
        if key.is_null() {
            return None;
        }
        let inner = self.lock();
        let subs = inner.by_session.get(session_id)?;
        for sub_id in subs {
            if let Some(e) = inner.by_id.get(sub_id) {
                let bound = e
                    .trigger_id
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .is_some();
                if bound && e.dedup_key == *key {
                    return Some(sub_id.clone());
                }
            }
        }
        None
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

    /// Claim a fired subscription against local liveness/ownership. One-shot
    /// fires remove the entry and return the trigger id for teardown; recurring
    /// fires keep the entry and return a monotonic notification entry id.
    pub fn claim_fire(&self, sub_id: &str, session_id: &str, once: bool) -> Option<FireClaim> {
        let mut inner = self.lock();
        let entry = inner.by_id.get(sub_id).cloned()?;
        if entry.session_id != session_id {
            return None;
        }

        if once {
            let entry = remove_entry(&mut inner, sub_id)?;
            return Some(FireClaim {
                entry_id: format!("e_notify_{sub_id}"),
                trigger_id: trigger_id(&entry),
            });
        }

        Some(FireClaim {
            entry_id: format!(
                "e_notify_{sub_id}_{}",
                entry.seq.fetch_add(1, Ordering::SeqCst) + 1
            ),
            trigger_id: None,
        })
    }

    /// The owning session of a subscription (unsubscribe ownership check).
    pub fn session_of(&self, sub_id: &str) -> Option<String> {
        self.lock().by_id.get(sub_id).map(|e| e.session_id.clone())
    }

    /// The engine trigger id bound to a subscription, WITHOUT evicting the
    /// slot. `None` for an unknown id or one still in the bind window (no
    /// engine id recorded yet) — callers distinguish via `session_of`.
    pub fn trigger_id_of(&self, sub_id: &str) -> Option<String> {
        self.lock().by_id.get(sub_id).and_then(|e| trigger_id(e))
    }

    /// Atomically remove and return `(session_id, trigger_id)`. Winner-takes-all:
    /// only the first caller for a given id gets `Some` — this is the `once` /
    /// unsubscribe claim that guarantees at-most-once delivery + teardown.
    pub fn take(&self, sub_id: &str) -> Option<(String, Option<String>)> {
        let mut inner = self.lock();
        let entry = remove_entry(&mut inner, sub_id)?;
        Some((entry.session_id.clone(), trigger_id(&entry)))
    }

    /// Remove the subscription bound to an ENGINE trigger id and return its
    /// sub id. Used by react's join teardown: the engine binding is already
    /// unregistered there, so the local slot must be evicted too or fired
    /// joins would permanently leak the owning session's cap budget. Linear
    /// scan — the map is bounded (per-session cap) and this runs only on
    /// join teardown.
    pub fn take_by_trigger_id(&self, trigger_id: &str) -> Option<String> {
        let mut inner = self.lock();
        let sub_id = inner.by_id.iter().find_map(|(id, e)| {
            let t = e.trigger_id.lock().unwrap_or_else(|p| p.into_inner());
            (t.as_deref() == Some(trigger_id)).then(|| id.clone())
        })?;
        remove_entry(&mut inner, &sub_id)?;
        Some(sub_id)
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
        assert!(reg.try_insert("sub_1", "s", 2).is_ok());
        assert!(reg.try_insert("sub_2", "s", 2).is_ok());
        assert!(reg.try_insert("sub_3", "s", 2).is_err());
        // A different session has its own budget.
        assert!(reg.try_insert("sub_4", "s2", 2).is_ok());
    }

    #[test]
    fn session_expects_wake_tracks_only_wake_entries() {
        let reg = SubscriptionRegistry::new();
        // A standing watcher / react binding is not a wake.
        reg.try_insert("sub_watch", "s", 8).unwrap();
        assert!(!reg.session_expects_wake("s"));
        // A one-shot notify is.
        reg.try_insert_keyed("sub_wake", "s", 8, serde_json::Value::Null, true)
            .unwrap();
        assert!(reg.session_expects_wake("s"));
        assert!(!reg.session_expects_wake("other"));
        // Consuming the one-shot fire clears the expectation.
        reg.set_trigger_id("sub_wake", "trig_1");
        assert!(reg.claim_fire("sub_wake", "s", true).is_some());
        assert!(!reg.session_expects_wake("s"));
    }

    #[test]
    fn find_duplicate_matches_only_bound_identical_same_session_requests() {
        let reg = SubscriptionRegistry::new();
        let key = serde_json::json!({"trigger_type":"state","config":{"scope":"a","key":"k"}});
        reg.try_insert_keyed("sub_1", "s", 8, key.clone(), false)
            .unwrap();
        // Unbound (engine registration in flight): allowed to race, no match.
        assert_eq!(reg.find_duplicate("s", &key), None);
        assert!(reg.set_trigger_id("sub_1", "t-1"));
        assert_eq!(reg.find_duplicate("s", &key).as_deref(), Some("sub_1"));
        // Different session or different request: no match.
        assert_eq!(reg.find_duplicate("s2", &key), None);
        assert_eq!(
            reg.find_duplicate("s", &serde_json::json!({"trigger_type":"cron"})),
            None
        );
        // Unkeyed inserts never dedup, even against a Null probe.
        reg.try_insert("sub_2", "s", 8).unwrap();
        assert!(reg.set_trigger_id("sub_2", "t-2"));
        assert_eq!(reg.find_duplicate("s", &serde_json::Value::Null), None);
    }

    #[test]
    fn set_trigger_id_then_take_returns_it() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "s", 8).unwrap();
        assert!(reg.set_trigger_id("sub_1", "trig_1"));
        let (session, trigger_id) = reg.take("sub_1").expect("entry present");
        assert_eq!(session, "s");
        assert_eq!(trigger_id.as_deref(), Some("trig_1"));
    }

    #[test]
    fn trigger_id_of_reads_without_evicting() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "s", 8).unwrap();
        // Bind window: no engine id recorded yet, but the slot exists.
        assert_eq!(reg.trigger_id_of("sub_1"), None);
        assert_eq!(reg.session_of("sub_1").as_deref(), Some("s"));
        assert!(reg.set_trigger_id("sub_1", "trig_1"));
        assert_eq!(reg.trigger_id_of("sub_1").as_deref(), Some("trig_1"));
        // Non-destructive: the slot is still takeable afterwards.
        assert!(reg.take("sub_1").is_some());
        assert_eq!(reg.trigger_id_of("sub_1"), None);
    }

    #[test]
    fn set_trigger_id_false_after_take() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "s", 8).unwrap();
        assert!(reg.take("sub_1").is_some());
        // Entry gone (a once-fire won the window): caller must unregister the orphan.
        assert!(!reg.set_trigger_id("sub_1", "trig_1"));
    }

    #[test]
    fn claim_recurring_fire_is_monotonic_and_owner_checked() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "owner", 8).unwrap();
        assert_eq!(reg.claim_fire("sub_1", "other", false), None);
        assert_eq!(
            reg.claim_fire("sub_1", "owner", false),
            Some(FireClaim {
                entry_id: "e_notify_sub_1_1".to_string(),
                trigger_id: None,
            })
        );
        assert_eq!(
            reg.claim_fire("sub_1", "owner", false),
            Some(FireClaim {
                entry_id: "e_notify_sub_1_2".to_string(),
                trigger_id: None,
            })
        );
        reg.take("sub_1");
        assert_eq!(reg.claim_fire("sub_1", "owner", false), None);
    }

    #[test]
    fn take_is_idempotent_winner_takes_all() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "s", 8).unwrap();
        assert!(reg.take("sub_1").is_some());
        assert!(reg.take("sub_1").is_none());
    }

    #[test]
    fn session_of_and_take_session() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "s", 8).unwrap();
        reg.try_insert("sub_2", "s", 8).unwrap();
        reg.set_trigger_id("sub_1", "trig_1");
        assert_eq!(reg.session_of("sub_1").as_deref(), Some("s"));
        let mut dropped = reg.take_session("s");
        dropped.sort();
        assert_eq!(dropped.len(), 2);
        assert!(reg.session_of("sub_1").is_none());
        assert!(reg.take_session("s").is_empty());
    }

    #[test]
    fn take_by_trigger_id_evicts_the_bound_slot_and_frees_cap() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "s", 1).unwrap();
        reg.set_trigger_id("sub_1", "trig_1");

        assert_eq!(reg.take_by_trigger_id("trig_1").as_deref(), Some("sub_1"));
        assert!(reg.session_of("sub_1").is_none());
        // The freed slot must count against the cap again.
        assert!(reg.try_insert("sub_2", "s", 1).is_ok());
        // Unknown / already-evicted trigger ids are a no-op.
        assert!(reg.take_by_trigger_id("trig_1").is_none());
        assert!(reg.take_by_trigger_id("trig_unknown").is_none());
    }

    #[test]
    fn claim_once_fire_removes_only_for_matching_owner() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "owner", 8).unwrap();
        reg.set_trigger_id("sub_1", "trig_1");

        assert_eq!(reg.claim_fire("sub_1", "other", true), None);
        assert_eq!(reg.session_of("sub_1").as_deref(), Some("owner"));
        assert_eq!(
            reg.claim_fire("sub_1", "owner", true),
            Some(FireClaim {
                entry_id: "e_notify_sub_1".to_string(),
                trigger_id: Some("trig_1".to_string()),
            })
        );
        assert!(reg.session_of("sub_1").is_none());
    }
}
