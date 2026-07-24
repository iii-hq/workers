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

/// Reaction-spec fingerprint of a live join predecessor. All predecessors of
/// one join id must agree on it — the join fires with the COMPLETING
/// predecessor's spec, so a divergent spec on any predecessor would silently
/// replace the real reaction at fire time.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinProbe {
    /// The shared join id (the accumulator record key — globally namespaced).
    pub id: String,
    /// The canonicalized reaction spec minus per-predecessor fields (see
    /// `react::join_canon`).
    pub canon: Value,
}

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
    /// Set post-insert for join-predecessor react bindings (see
    /// [`set_join_probe`](SubscriptionRegistry::set_join_probe)); dies with
    /// the entry, so conflict checks never see retired predecessors.
    join: Mutex<Option<JoinProbe>>,
}

impl SubEntry {
    fn new(session_id: String, dedup_key: Value, wake: bool) -> Self {
        Self {
            session_id,
            seq: AtomicU64::new(0),
            trigger_id: Mutex::new(None),
            dedup_key,
            wake,
            join: Mutex::new(None),
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
    /// Reaction session → registrant session, recorded when a subscription's
    /// fire spawns into a distinct session. Lets a reaction clean up its own
    /// run: unregistering a subscription is permitted for any session whose
    /// lineage chain reaches the owner (the registrant may be parked waiting
    /// on a wake its children must tear down). Ephemeral like the rest of the
    /// registry; entries are purged on `session::deleted`.
    lineage: HashMap<String, String>,
    /// Lifetime delivered-fire counts for budgeted react bindings
    /// (`max_fires`), keyed by subscription id — or by the react gate's
    /// spec-hash fallback for raw engine-side registrations that carry no
    /// `__subscription_id`. Counters die with their entry; fallback keys are
    /// pruned once the map outgrows [`MAX_FIRE_COUNTERS`].
    fire_counts: HashMap<String, u64>,
}

/// Bound on [`Inner::fire_counts`] growth from spec-hash fallback keys, which
/// have no eviction event of their own. Far above any realistic number of
/// distinct live budgeted bindings.
const MAX_FIRE_COUNTERS: usize = 4096;

/// Upper bound when walking the lineage chain — bounds cycles (a session id
/// can be re-targeted across runs) and pathological depth alike. Deeper than
/// any real reactive chain, which is itself capped by the reactive-depth
/// breaker.
const MAX_LINEAGE_HOPS: usize = 16;

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

    /// Attach a join-predecessor fingerprint after a successful insert. Same
    /// race posture as [`set_trigger_id`](Self::set_trigger_id): `false` means
    /// the entry is already gone.
    pub fn set_join_probe(&self, sub_id: &str, probe: JoinProbe) -> bool {
        match self.lock().by_id.get(sub_id) {
            Some(entry) => {
                *entry.join.lock().unwrap_or_else(|p| p.into_inner()) = Some(probe);
                true
            }
            None => false,
        }
    }

    /// Find a LIVE join predecessor of `join_id` whose reaction spec differs
    /// from `canon`. Join ids are globally namespaced (the accumulator record
    /// key), so this scans across sessions. `Some(sub_id)` names the
    /// conflicting predecessor for the registration error.
    pub fn conflicting_join_spec(&self, join_id: &str, canon: &Value) -> Option<String> {
        let inner = self.lock();
        for (sub_id, entry) in inner.by_id.iter() {
            let guard = entry.join.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(probe) = guard.as_ref() {
                if probe.id == join_id && probe.canon != *canon {
                    return Some(sub_id.clone());
                }
            }
        }
        None
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

    /// Count one delivered fire for a budgeted (`max_fires`) binding and
    /// return the new lifetime total. `key` is the subscription id, or the
    /// react gate's spec-hash fallback for raw bindings; counters for
    /// registered subscriptions are dropped with their entry, fallback keys
    /// are pruned when the map outgrows its bound.
    pub fn record_budgeted_fire(&self, key: &str) -> u64 {
        let mut inner = self.lock();
        if inner.fire_counts.len() >= MAX_FIRE_COUNTERS && !inner.fire_counts.contains_key(key) {
            // Only fallback keys can accumulate without an eviction event:
            // keep counters that still back a live subscription entry.
            let by_id = std::mem::take(&mut inner.by_id);
            inner.fire_counts.retain(|k, _| by_id.contains_key(k));
            inner.by_id = by_id;
        }
        let count = inner.fire_counts.entry(key.to_string()).or_insert(0);
        *count += 1;
        *count
    }

    /// Record that `child_session` was spawned by a reaction of a subscription
    /// registered by `registrant_session`. Overwrites any prior parent — the
    /// latest spawner is the live lineage.
    pub fn record_lineage(&self, child_session: &str, registrant_session: &str) {
        if child_session == registrant_session {
            return;
        }
        self.lock()
            .lineage
            .insert(child_session.to_string(), registrant_session.to_string());
    }

    /// Whether `caller`'s lineage chain (reaction session → registrant,
    /// transitively) reaches `owner`. Grants a reaction the right to
    /// unregister its run's subscriptions even though another session
    /// registered them.
    pub fn in_lineage_of(&self, caller: &str, owner: &str) -> bool {
        let inner = self.lock();
        let mut current = caller;
        for _ in 0..MAX_LINEAGE_HOPS {
            match inner.lineage.get(current) {
                Some(parent) if parent == owner => return true,
                Some(parent) => current = parent,
                None => return false,
            }
        }
        false
    }

    /// Drop lineage entries pointing at or from a deleted session so the map
    /// stays bounded by live sessions.
    pub fn forget_lineage(&self, session_id: &str) {
        let mut inner = self.lock();
        inner.lineage.remove(session_id);
        inner.lineage.retain(|_, parent| parent != session_id);
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
    inner.fire_counts.remove(sub_id);
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
    fn join_probe_conflict_detection_and_lifecycle() {
        let reg = SubscriptionRegistry::new();
        let canon = serde_json::json!({"task": "the real task", "model": "m"});
        reg.try_insert("sub_w1", "s", 8).unwrap();
        assert!(reg.set_join_probe(
            "sub_w1",
            JoinProbe {
                id: "j1".into(),
                canon: canon.clone()
            }
        ));

        // Identical spec on another predecessor: no conflict.
        assert!(reg.conflicting_join_spec("j1", &canon).is_none());
        // Divergent spec (the "placeholder" shape): conflict names the live twin.
        let placeholder = serde_json::json!({"task": "placeholder", "model": "m"});
        assert_eq!(
            reg.conflicting_join_spec("j1", &placeholder).as_deref(),
            Some("sub_w1")
        );
        // A different join id is unaffected.
        assert!(reg.conflicting_join_spec("j2", &placeholder).is_none());

        // The probe dies with the entry — retired predecessors never conflict.
        assert!(reg.take("sub_w1").is_some());
        assert!(reg.conflicting_join_spec("j1", &placeholder).is_none());
        // And a probe cannot attach to a gone entry.
        assert!(!reg.set_join_probe(
            "sub_w1",
            JoinProbe {
                id: "j1".into(),
                canon
            }
        ));
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
    fn lineage_grants_transitive_descendants_and_nothing_else() {
        let reg = SubscriptionRegistry::new();
        // orchestrator → reactor → repair (a reaction spawning a reaction).
        reg.record_lineage("reactor", "orchestrator");
        reg.record_lineage("repair", "reactor");

        assert!(reg.in_lineage_of("reactor", "orchestrator"));
        assert!(reg.in_lineage_of("repair", "orchestrator"), "transitive");
        assert!(reg.in_lineage_of("repair", "reactor"));
        // Never the other direction, never a stranger.
        assert!(!reg.in_lineage_of("orchestrator", "reactor"));
        assert!(!reg.in_lineage_of("stranger", "orchestrator"));
        assert!(!reg.in_lineage_of("reactor", "repair"));
    }

    #[test]
    fn lineage_self_edges_are_not_recorded_and_cycles_terminate() {
        let reg = SubscriptionRegistry::new();
        reg.record_lineage("s", "s");
        assert!(!reg.in_lineage_of("s", "s"), "self edge is meaningless");
        // A ↔ B cycle (session ids re-targeted across runs) must terminate
        // instead of spinning, and must not grant unrelated ownership.
        reg.record_lineage("a", "b");
        reg.record_lineage("b", "a");
        assert!(!reg.in_lineage_of("a", "z"));
        assert!(reg.in_lineage_of("a", "b"));
    }

    #[test]
    fn forget_lineage_purges_both_directions() {
        let reg = SubscriptionRegistry::new();
        reg.record_lineage("child", "parent");
        reg.record_lineage("grandchild", "child");
        reg.forget_lineage("child");
        assert!(!reg.in_lineage_of("child", "parent"));
        assert!(
            !reg.in_lineage_of("grandchild", "child"),
            "entries pointing AT the deleted session are purged too"
        );
    }

    #[test]
    fn budgeted_fire_counts_are_monotonic_and_die_with_their_entry() {
        let reg = SubscriptionRegistry::new();
        reg.try_insert("sub_1", "s", 8).unwrap();
        assert_eq!(reg.record_budgeted_fire("sub_1"), 1);
        assert_eq!(reg.record_budgeted_fire("sub_1"), 2);
        // Independent keys have independent budgets (spec-hash fallback).
        assert_eq!(reg.record_budgeted_fire("spec:abc"), 1);
        // Retiring the entry resets its counter — a re-registered binding
        // under a fresh sub id starts a fresh budget anyway, but a reused id
        // must not inherit spent fires.
        reg.take("sub_1");
        assert_eq!(reg.record_budgeted_fire("sub_1"), 1);
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
