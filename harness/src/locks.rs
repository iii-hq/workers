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

#[cfg(test)]
mod tests {
    use super::SessionLocks;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;

    /// The property the abort-write fix relies on: two holders of the SAME
    /// session lock never overlap, so a read-modify-write under the guard can't
    /// be clobbered by a concurrent one (e.g. harness::stop vs the turn loop).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn same_session_critical_sections_never_overlap() {
        let locks = SessionLocks::new();
        let inside = Arc::new(AtomicBool::new(false));
        let overlaps = Arc::new(AtomicU32::new(0));

        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let (locks, inside, overlaps) = (locks.clone(), inside.clone(), overlaps.clone());
                tokio::spawn(async move {
                    let _g = locks.guard("s").await;
                    // If mutual exclusion holds, `inside` is always false here.
                    if inside.swap(true, Ordering::SeqCst) {
                        overlaps.fetch_add(1, Ordering::SeqCst);
                    }
                    tokio::task::yield_now().await; // widen the window for a racer
                    inside.store(false, Ordering::SeqCst);
                })
            })
            .collect();
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(
            overlaps.load(Ordering::SeqCst),
            0,
            "critical sections overlapped"
        );
    }

    /// Different sessions must NOT serialize — holding one session's guard can't
    /// block another session's stop/step.
    #[tokio::test]
    async fn distinct_sessions_do_not_block_each_other() {
        let locks = SessionLocks::new();
        let held = locks.guard("a").await;
        // A different session acquires without waiting on `held`.
        let _other = locks.guard("b").await;
        drop(held);
    }
}
