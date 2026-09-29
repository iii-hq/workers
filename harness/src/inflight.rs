//! Orphaned-turn recovery.
//!
//! A turn record can be `Running` with no `harness::turn` step on the queue:
//! the enqueue that should carry its current step failed (queue worker down,
//! namespace-unaware queue provider, engine restart) after the record was
//! persisted. Nothing would ever run that step again, so the session stays
//! "working" forever and even `harness::stop` cannot finish it — stop only
//! sets the abort bit the next step observes.
//!
//! [`InflightSteps`] tracks the sessions whose step is executing in this
//! process right now. [`redrive_orphans`] (run by [`run_loop`] and the pending
//! sweep) re-enqueues
//! the current step of every `Running` turn that has not moved for
//! [`ORPHAN_REDRIVE_AFTER_MS`] and is not executing here. Re-enqueueing is
//! safe because steps are at-least-once: the queue is FIFO per `session_id`
//! and `generate_step` acks any delivery whose `(turn_id, step)` is no longer
//! current, so a duplicate of a step that was only delayed is dropped.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::types::message::AgentMessage;
use crate::types::turn::{TurnRecord, TurnStatus};

/// How long a `Running` turn may go without a record update, while no step
/// for it executes in this process, before its current step is re-enqueued.
pub const ORPHAN_REDRIVE_AFTER_MS: u64 = 120_000;

/// Sessions whose `harness::turn` step is executing in this process.
#[derive(Clone, Default)]
pub struct InflightSteps {
    inner: Arc<Mutex<HashMap<String, usize>>>,
}

/// Marks a session's step as executing until dropped.
pub struct InflightGuard {
    steps: InflightSteps,
    session_id: String,
}

impl InflightSteps {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `session_id` as executing a step until the guard drops.
    pub fn enter(&self, session_id: &str) -> InflightGuard {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        *map.entry(session_id.to_string()).or_insert(0) += 1;
        InflightGuard {
            steps: self.clone(),
            session_id: session_id.to_string(),
        }
    }

    pub fn contains(&self, session_id: &str) -> bool {
        let map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.get(session_id).is_some_and(|n| *n > 0)
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        let mut map = self.steps.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = map.get_mut(&self.session_id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                map.remove(&self.session_id);
            }
        }
    }
}

/// Whether `record` looks orphaned: `Running`, unchanged for the redrive
/// window, and not executing a step in this process.
pub fn is_orphan_candidate(record: &TurnRecord, executing_here: bool, now: i64) -> bool {
    record.status == TurnStatus::Running
        && !executing_here
        && now.saturating_sub(record.updated_at) >= ORPHAN_REDRIVE_AFTER_MS as i64
}

/// Re-enqueue the current step of a turn whose step is not executing in this
/// process. Used by `harness::stop` so a stop on an orphaned turn is observed.
pub async fn redrive_if_idle(deps: &Deps, record: &TurnRecord) -> Result<bool, HarnessError> {
    if record.status != TurnStatus::Running || deps.inflight.contains(&record.session_id) {
        return Ok(false);
    }
    crate::turn_loop::enqueue_step(
        &deps.iii,
        &record.session_id,
        &record.turn_id,
        record.step,
        record.message_preview.as_deref(),
        record.depth,
    )
    .await?;
    Ok(true)
}

/// Re-enqueue the current step of every orphaned `Running` turn. Returns the
/// number of steps re-enqueued.
pub async fn redrive_orphans(deps: &Deps) -> Result<u64, HarnessError> {
    let cfg = deps.cfg().await;
    let records = crate::state::list_turns(&deps.iii, cfg.session_timeout_ms).await?;
    let now = AgentMessage::now_ms();
    let mut redriven = 0;
    for mut record in records {
        if !is_orphan_candidate(&record, deps.inflight.contains(&record.session_id), now) {
            continue;
        }
        // Re-check under the session lock against the freshest record so a
        // step that just advanced or finished is not redriven.
        let _guard = deps.locks.guard(&record.session_id).await;
        match crate::state::get_turn(&deps.iii, &record.session_id, cfg.session_timeout_ms).await? {
            Some(fresh)
                if fresh.turn_id == record.turn_id
                    && fresh.step == record.step
                    && is_orphan_candidate(
                        &fresh,
                        deps.inflight.contains(&fresh.session_id),
                        now,
                    ) =>
            {
                record = fresh;
            }
            _ => continue,
        }
        match redrive_if_idle(deps, &record).await {
            Ok(true) => {
                // Restart the window so the next sweep does not re-enqueue a
                // step that is merely waiting its turn on the queue.
                record.updated_at = now;
                let _ = crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await;
                tracing::warn!(
                    session_id = %record.session_id,
                    turn_id = %record.turn_id,
                    step = record.step,
                    "re-enqueued the step of an orphaned running turn"
                );
                redriven += 1;
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(
                session_id = %record.session_id,
                turn_id = %record.turn_id,
                error = %e,
                "could not re-enqueue an orphaned running turn"
            ),
        }
    }
    Ok(redriven)
}

/// Delay before the first orphan pass after boot, so the queue worker and the
/// harness's own registrations are back before anything is re-enqueued.
const BOOT_DELAY_MS: u64 = 30_000;

/// Background loop: one orphan pass shortly after boot (turns stranded by the
/// outage that preceded a restart), then one per redrive window. The daily
/// pending sweep alone would leave a wedged session stuck for up to a day.
pub async fn run_loop(deps: std::sync::Arc<Deps>) {
    tokio::time::sleep(std::time::Duration::from_millis(BOOT_DELAY_MS)).await;
    loop {
        if let Err(e) = redrive_orphans(&deps).await {
            tracing::warn!(error = %e, "orphaned-turn redrive pass failed");
        }
        tokio::time::sleep(std::time::Duration::from_millis(ORPHAN_REDRIVE_AFTER_MS)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_tracks_nested_entries_per_session() {
        let steps = InflightSteps::new();
        assert!(!steps.contains("s"));
        let a = steps.enter("s");
        let b = steps.enter("s");
        assert!(steps.contains("s"));
        drop(a);
        assert!(
            steps.contains("s"),
            "a second entry keeps the session in flight"
        );
        drop(b);
        assert!(!steps.contains("s"));
        assert!(!steps.contains("other"));
    }

    fn record(status: &str, updated_at: i64) -> TurnRecord {
        serde_json::from_value(serde_json::json!({
            "turn_id": "t_1", "session_id": "s_1", "status": status,
            "step": 0, "turn_count": 0, "depth": 0,
            "options": { "model": "m", "max_turns": 16 },
            "created_at": updated_at, "updated_at": updated_at
        }))
        .unwrap()
    }

    #[test]
    fn only_stale_running_turns_idle_here_are_orphans() {
        let window = ORPHAN_REDRIVE_AFTER_MS as i64;
        let now = 10 * window;
        let stale = now - window;
        assert!(is_orphan_candidate(&record("running", stale), false, now));
        assert!(
            !is_orphan_candidate(&record("running", stale), true, now),
            "a step executing in this process is never redriven"
        );
        assert!(
            !is_orphan_candidate(&record("running", now - window + 1), false, now),
            "a recently updated turn may still have its step queued"
        );
        for status in ["awaiting_functions", "completed", "cancelled", "failed"] {
            assert!(
                !is_orphan_candidate(&record(status, stale), false, now),
                "{status}"
            );
        }
    }
}
