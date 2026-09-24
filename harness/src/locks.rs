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
///
/// The same primitive backs `Deps::call_cancels`, the per-call signals
/// `harness::function::cancel` fires: there the key is [`call_cancel_key`]
/// (turn + call id) and the tool phase races the in-flight target invocation
/// against `wait_fired` on it.
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

    /// Drop every per-call signal of `turn_id` (keys from [`call_cancel_key`])
    /// once the turn is terminal: a cancel fired for a call the loop never
    /// raced (finalized first, or a locally intercepted control call) would
    /// otherwise linger.
    pub fn clear_turn_calls(&self, turn_id: &str) {
        let prefix = call_cancel_key(turn_id, "");
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|key, _| !key.starts_with(&prefix));
    }
}

/// The `Deps::call_cancels` key for one function call: scoped by turn so a
/// provider call id reused across turns (or sessions) never shares a signal.
pub fn call_cancel_key(turn_id: &str, function_call_id: &str) -> String {
    format!("{turn_id}/{function_call_id}")
}

/// Resolve once `rx`'s signal is fired. Level-triggered: a fire before the
/// call is observed immediately. A signal whose sender was dropped (the key
/// was cleared) stays pending forever — a cleared key must never read as a
/// cancel, so a `select!` racing this against a target invocation always
/// lets the real result win.
pub async fn wait_fired(rx: &mut tokio::sync::watch::Receiver<bool>) {
    if rx.wait_for(|fired| *fired).await.is_err() {
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
mod tests {
    use super::{call_cancel_key, wait_fired, SessionLocks, TurnCancels};
    use crate::types::turn::SkillContext;
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

    #[tokio::test]
    async fn post_generation_refresh_keeps_a_filter_written_in_the_unlocked_window() {
        let locks = SessionLocks::new();
        let initial_guard = locks.guard("s").await;
        let durable = Arc::new(tokio::sync::Mutex::new(SkillContext {
            filter: Some(vec!["old".into()]),
            baseline: Some("durable baseline".into()),
        }));
        let writer_locks = locks.clone();
        let writer_durable = durable.clone();
        let writer = tokio::spawn(async move {
            let _guard = writer_locks.guard("s").await;
            writer_durable.lock().await.filter = Some(vec!["new".into()]);
        });

        let mut in_memory = Some(SkillContext {
            filter: Some(vec!["old".into()]),
            baseline: Some("generation baseline".into()),
        });
        drop(initial_guard);
        writer.await.unwrap();

        let _post_generation_guard = locks.guard("s").await;
        let durable = durable.lock().await;
        crate::skills::refresh_filter(&mut in_memory, Some(&durable));

        assert_eq!(in_memory.as_ref().unwrap().filter, Some(vec!["new".into()]));
        assert_eq!(
            in_memory.as_ref().unwrap().baseline.as_deref(),
            Some("generation baseline")
        );
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

    /// A per-call key is scoped by turn: the same provider call id in two
    /// turns (or sessions) never shares a signal, so a stale cancel from an
    /// earlier turn can't interrupt a later call.
    #[test]
    fn call_cancel_keys_are_scoped_by_turn() {
        assert_eq!(call_cancel_key("t1", "call_a"), "t1/call_a");
        assert_ne!(
            call_cancel_key("t1", "call_a"),
            call_cancel_key("t2", "call_a")
        );
        assert_ne!(
            call_cancel_key("t1", "call_a"),
            call_cancel_key("t1", "call_b")
        );
    }

    /// Finalizing a turn drops its per-call signals and nobody else's.
    #[test]
    fn clear_turn_calls_drops_only_that_turns_call_signals() {
        let cancels = TurnCancels::new();
        cancels.fire(&call_cancel_key("t1", "call_a"));
        cancels.fire(&call_cancel_key("t1", "call_b"));
        cancels.fire(&call_cancel_key("t10", "call_a"));
        cancels.clear_turn_calls("t1");
        assert!(!cancels.is_fired(&call_cancel_key("t1", "call_a")));
        assert!(!cancels.is_fired(&call_cancel_key("t1", "call_b")));
        assert!(cancels.is_fired(&call_cancel_key("t10", "call_a")));
    }

    /// `wait_fired` resolves once the signal is set — including when it was
    /// set BEFORE the wait began (level-triggered, like the chat backstop).
    #[tokio::test]
    async fn wait_fired_resolves_on_a_prior_or_later_fire() {
        let cancels = TurnCancels::new();
        cancels.fire("early");
        let mut rx = cancels.watch("early");
        tokio::time::timeout(std::time::Duration::from_millis(100), wait_fired(&mut rx))
            .await
            .expect("a prior fire is observed immediately");

        let mut rx = cancels.watch("late");
        let waiter = tokio::spawn(async move { wait_fired(&mut rx).await });
        tokio::task::yield_now().await;
        cancels.fire("late");
        tokio::time::timeout(std::time::Duration::from_millis(100), waiter)
            .await
            .expect("a later fire wakes the waiter")
            .unwrap();
    }

    /// Clearing a key the loop is still racing against must NOT read as a
    /// cancel: the waiter stays pending so the real call result wins.
    #[tokio::test]
    async fn wait_fired_stays_pending_when_the_signal_is_cleared() {
        let cancels = TurnCancels::new();
        let mut rx = cancels.watch("gone");
        cancels.clear("gone");
        let outcome =
            tokio::time::timeout(std::time::Duration::from_millis(50), wait_fired(&mut rx)).await;
        assert!(
            outcome.is_err(),
            "a cleared signal must never resolve as fired"
        );
    }
}
