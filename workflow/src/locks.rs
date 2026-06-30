//! Per-run in-process serialization. `workflow::tick`, `reconcile`, the sweep,
//! and `workflow::stop` all run off the queue and write the same run record;
//! guarding them with one per-run lock closes the read-modify-write race within
//! a single process.
//!
//! NOTE: this is single-process correctness. A multi-process deployment needs
//! an engine-level compare-and-set on the run record (which iii-state does NOT
//! provide); the fail-safe is the deterministic child-session id and
//! `workflow_node_result` key, which keeps duplicate deliveries idempotent.
//! For multi-instance HA, shard `workflow::tick` by `run_id` so one owning
//! instance handles all writes for a given run — no new code required.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::OwnedMutexGuard;

#[derive(Clone, Default)]
pub struct WorkflowLocks {
    map: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl WorkflowLocks {
    /// Acquire the lock for `run_id`, creating it on first use.
    pub async fn guard(&self, run_id: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
            map.entry(run_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }
}
