//! Per-run in-process serialization. `workflow::tick`, `reconcile`, the sweep,
//! and `workflow::stop` all run off the queue and write the same run record;
//! guarding them with one per-run lock closes the read-modify-write race within
//! a single process.
//!
//! The fixed key `"idem"` (`start::IDEM_LOCK`) also serializes idempotency-key
//! check-and-set against its GC delete.
//!
//! NOTE: this is single-process correctness, which holds because the engine
//! allows one instance per namespace. `state::compare-and-set` exists but is not
//! adopted (whole-value compare, absent equals stored null, no size guard,
//! scope-wide WATCH with 8 attempts on Redis); the fail-safe is the
//! deterministic child-session id and `workflow_node_result` key, which keeps
//! duplicate deliveries idempotent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::OwnedMutexGuard;

#[derive(Clone, Default)]
pub struct WorkflowLocks {
    map: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
}

impl WorkflowLocks {
    /// Acquire the lock for `run_id`, creating it on first use.
    pub async fn guard(&self, run_id: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
            // Reuse the live lock while any guard is still outstanding; once all
            // guards drop, the Weak lapses and we mint a fresh one. Storing Weak
            // (not Arc) lets a finished run's mutex deallocate instead of pinning
            // it in the map forever.
            // ponytail: a lapsed Weak entry still lingers per distinct run_id (tiny
            // — a key + 16-byte Weak, no mutex). Add a periodic `retain(|_, w|
            // w.strong_count() > 0)` prune only if run-id cardinality makes even
            // that matter.
            match map.get(run_id).and_then(Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    let lock = Arc::new(tokio::sync::Mutex::new(()));
                    map.insert(run_id.to_string(), Arc::downgrade(&lock));
                    lock
                }
            }
        };
        lock.lock_owned().await
    }
}
