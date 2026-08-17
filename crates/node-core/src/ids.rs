//! Which runtime owns each registered function id.
//!
//! `iii-sdk`'s `register_function_inner` panics when an id is already
//! registered on the same client (`iii.rs:944`). That call happens inside
//! `op_iii_register`, a `#[op2(fast)]` V8 callback with an `extern "C"` ABI —
//! and unwinding out of `extern "C"` aborts. A duplicate id therefore kills
//! the process and every tenant's runtime with it, so every path that reaches
//! `Engine::register` must claim here first.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Owner recorded for ids the worker registers itself. `create` generates
/// runtime ids as `rt-<uuid>`, so no runtime can equal this and
/// `release_owner` can never free a worker id.
pub const WORKER_OWNER: &str = "<worker>";

/// id -> owning runtime id, or [`WORKER_OWNER`].
///
/// Cheap to clone: every runtime holds the same map.
#[derive(Clone, Default)]
pub struct IdRegistry(Arc<Mutex<HashMap<String, String>>>);

impl IdRegistry {
    /// Seed with the ids this worker registers itself. Omitting one is a
    /// process abort waiting to happen — see the module comment.
    pub fn with_worker_ids(ids: &[&str]) -> Self {
        let map = ids
            .iter()
            .map(|id| ((*id).to_string(), WORKER_OWNER.to_string()))
            .collect();
        IdRegistry(Arc::new(Mutex::new(map)))
    }

    /// Claim one id. True when it was unclaimed or already owned by `owner`.
    pub fn claim(&self, id: &str, owner: &str) -> bool {
        let mut map = self.0.lock().unwrap();
        match map.get(id) {
            Some(current) if current != owner => false,
            _ => {
                map.insert(id.to_string(), owner.to_string());
                true
            }
        }
    }

    /// Claim every id or none. `Err` carries the first conflicting id.
    ///
    /// One lock for the check and the inserts, so two concurrent batches
    /// cannot both observe an id as free.
    pub fn claim_all(&self, ids: &[String], owner: &str) -> Result<(), String> {
        let mut map = self.0.lock().unwrap();
        if let Some(taken) = ids
            .iter()
            .find(|id| map.get(*id).is_some_and(|held| held != owner))
        {
            return Err(taken.clone());
        }
        for id in ids {
            map.insert(id.clone(), owner.to_string());
        }
        Ok(())
    }

    /// Free `ids` still held by `owner`. Two callers, two different reasons
    /// the id is safe to free: the failure arm of `RuntimeManager::register`
    /// releases ids that provably never reached the bus, and
    /// `OpsState::unregister` (ops.rs) releases an id that DID reach the bus
    /// but was just explicitly unpublished by its own owner — the `owner`
    /// check above is what makes either caller safe, not which one is
    /// calling.
    pub fn release_ids(&self, ids: &[String], owner: &str) {
        let mut map = self.0.lock().unwrap();
        for id in ids {
            if map.get(id).is_some_and(|held| held == owner) {
                map.remove(id);
            }
        }
    }

    /// Free everything owned by `owner`. Called from `RuntimeManager::destroy`
    /// — teardown, reap, and the idle sweep — and nowhere else.
    pub fn release_owner(&self, owner: &str) {
        self.0.lock().unwrap().retain(|_, held| held != owner);
    }
}

// Never the map itself: the values are runtime ids, which are capabilities.
// `RuntimeOpts` derives `Debug`, so this type needs one.
impl std::fmt::Debug for IdRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IdRegistry({} ids)", self.0.lock().unwrap().len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_an_unclaimed_id() {
        let reg = IdRegistry::default();
        assert!(reg.claim("app::a", "rt-1"));
    }

    #[test]
    fn re_claiming_your_own_id_succeeds() {
        let reg = IdRegistry::default();
        assert!(reg.claim("app::a", "rt-1"));
        // Preserves today's "re-registering swaps only the JS handler".
        assert!(reg.claim("app::a", "rt-1"));
    }

    #[test]
    fn another_owner_cannot_claim() {
        let reg = IdRegistry::default();
        assert!(reg.claim("app::a", "rt-1"));
        assert!(!reg.claim("app::a", "rt-2"));
    }

    #[test]
    fn worker_ids_are_seeded_and_unclaimable() {
        let reg = IdRegistry::with_worker_ids(&["node-engine::eval"]);
        assert!(!reg.claim("node-engine::eval", "rt-1"));
    }

    #[test]
    fn release_owner_frees_only_that_owners_ids() {
        let reg = IdRegistry::with_worker_ids(&["node-engine::eval"]);
        assert!(reg.claim("app::a", "rt-1"));
        assert!(reg.claim("app::b", "rt-2"));
        reg.release_owner("rt-1");
        assert!(reg.claim("app::a", "rt-2"), "rt-1's id should be free");
        assert!(
            !reg.claim("app::b", "rt-1"),
            "rt-2's id should still be held"
        );
        assert!(
            !reg.claim("node-engine::eval", "rt-1"),
            "a worker id must never be released by a runtime release"
        );
    }

    #[test]
    fn claim_all_is_atomic() {
        let reg = IdRegistry::default();
        assert!(reg.claim("app::b", "rt-other"));
        let ids = vec!["app::a".to_string(), "app::b".to_string()];
        assert_eq!(reg.claim_all(&ids, "rt-1"), Err("app::b".to_string()));
        // The conflict must have rolled back `app::a` too.
        assert!(reg.claim("app::a", "rt-2"), "partial claim leaked");
    }

    #[test]
    fn claim_all_tolerates_ids_you_already_own() {
        let reg = IdRegistry::default();
        assert!(reg.claim("app::a", "rt-1"));
        let ids = vec!["app::a".to_string(), "app::b".to_string()];
        assert_eq!(reg.claim_all(&ids, "rt-1"), Ok(()));
    }

    /// The map's values are runtime ids, which are capabilities. `RuntimeOpts`
    /// derives `Debug`, so this type needs one — and it must not print them.
    #[test]
    fn debug_does_not_leak_owners() {
        let reg = IdRegistry::default();
        assert!(reg.claim("app::a", "rt-secret"));
        let shown = format!("{reg:?}");
        assert!(!shown.contains("rt-secret"), "leaked an owner: {shown}");
        assert!(!shown.contains("app::a"), "leaked an id: {shown}");
    }
}
