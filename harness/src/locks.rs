//! Per-session in-process serialization. Turn steps on the `default` queue are
//! not per-session ordered, so the per-session lock serializes all turn-record
//! writers. `harness::function::resolve` and the pending sweep also run OFF the
//! queue (child completion, approval decisions, cron). Guarding both with one
//! per-session lock closes the read-modify-write race on the turn record within
//! a single process.
//!
//! NOTE: this is single-process correctness. A multi-process deployment needs
//! an engine-level compare-and-set on the turn record; the fail-safe is the
//! deterministic entry id, which keeps duplicate deliveries idempotent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::OwnedMutexGuard;

#[derive(Clone, Default)]
pub struct SessionLocks {
    map: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl SessionLocks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire the lock for `session_id`, creating it on first use.
    pub async fn guard(&self, session_id: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
            map.entry(session_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }
}
