//! Per-session in-process serialization. The `harness-turn` queue orders queued
//! steps by session, while `harness::function::resolve` and the pending sweep
//! also write turn records OFF the queue (child completion, approval decisions,
//! cron). Guarding every writer with one per-session lock closes their
//! read-modify-write race within a single process.
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

/// Per-turn in-process cancel signals. `harness::stop` fires one lock-free so
/// the running step can observe the stop where durable state can't reach it:
/// inside the `router::chat` await (via `watch`) and between tool executions
/// (via `is_fired`) — the tool phase holds the session lock, so the durable
/// abort write is blocked behind it by construction. Level-triggered `watch`
/// channels: a fire before subscribe is still observed. Keyed by turn_id so a
/// stale fire can never cancel a newer turn. Same single-process caveat as
/// `SessionLocks` above.
#[derive(Clone, Default)]
pub struct TurnCancels {
    map: Arc<Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>>,
}

impl TurnCancels {
    pub fn new() -> Self {
        Self::default()
    }

    /// Signal cancellation for `turn_id` (idempotent).
    pub fn fire(&self, turn_id: &str) {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.entry(turn_id.to_string())
            .or_insert_with(|| tokio::sync::watch::channel(false).0)
            .send_replace(true);
    }

    pub fn is_fired(&self, turn_id: &str) -> bool {
        let map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.get(turn_id).is_some_and(|s| *s.borrow())
    }

    /// Subscribe to `turn_id`'s cancel signal, creating it on first use.
    pub fn watch(&self, turn_id: &str) -> tokio::sync::watch::Receiver<bool> {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.entry(turn_id.to_string())
            .or_insert_with(|| tokio::sync::watch::channel(false).0)
            .subscribe()
    }

    /// Drop `turn_id`'s signal once the turn is terminal.
    pub fn clear(&self, turn_id: &str) {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(turn_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionLocks, TurnCancels};
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

    /// The property the chat-abort backstop relies on: level-triggering. A fire
    /// BEFORE anyone subscribes is still observed by a later watch/is_fired.
    #[tokio::test]
    async fn cancel_fired_before_subscribe_is_observed() {
        let cancels = TurnCancels::new();
        cancels.fire("t1");
        assert!(cancels.is_fired("t1"));
        let rx = cancels.watch("t1");
        assert!(*rx.borrow());
        assert!(!cancels.is_fired("t2"));
        cancels.clear("t1");
        assert!(!cancels.is_fired("t1"));
    }
}
