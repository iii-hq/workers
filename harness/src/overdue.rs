//! `harness::on-turn-overdue`: a turn still running at a check point —
//! `overdue_after_ms` after it was created, then every `overdue_every_ms` —
//! so a watchdog can judge long turns without polling them.
//!
//! Like the orphan follow-ups ([`crate::inflight`]), never a clock: each
//! running turn holds one deadline, armed by its first step and by the boot
//! read, dropped when the turn completes. At a deadline the record is re-read
//! and reported only when someone observes the event; either way the watch
//! moves to the next check point.
//!
//! `ponytail:` deadlines live in this process. After a restart a running turn
//! is reported from its next check point (one that passed while the harness
//! was down is skipped), and two harness processes would each report the
//! turns their boot reads saw. Persist the watches if that matters.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use crate::deps::Deps;
use crate::events::{OverdueCall, TurnOverdueEvent};
use crate::types::message::AgentMessage;
use crate::types::turn::{CallState, TurnRecord, TurnStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Watch {
    turn_id: String,
    created_at: i64,
    /// The next check point is the first one at or after this instant.
    from: i64,
}

#[derive(Default)]
struct Watches {
    entries: Mutex<BTreeMap<String, Watch>>,
    changed: tokio::sync::Notify,
}

static WATCHES: LazyLock<Watches> = LazyLock::new(Watches::default);

fn entries() -> MutexGuard<'static, BTreeMap<String, Watch>> {
    WATCHES.entries.lock().unwrap_or_else(|e| e.into_inner())
}

/// The first check point at or after `from` of a turn created at
/// `created_at`: `created_at + after`, then every `every`. `None` when the
/// event is off (`after == 0`) or its single check point (`every == 0`) is
/// behind `from`.
pub fn next_check_point(created_at: i64, from: i64, after: u64, every: u64) -> Option<i64> {
    if after == 0 {
        return None;
    }
    let first = created_at.saturating_add(after as i64);
    if from <= first {
        return Some(first);
    }
    if every == 0 {
        return None;
    }
    let every = every as i64;
    Some(first + (from - first + every - 1) / every * every)
}

fn is_running(status: TurnStatus) -> bool {
    matches!(status, TurnStatus::Running | TurnStatus::AwaitingFunctions)
}

fn watch(session_id: &str, turn_id: &str, created_at: i64, from: i64) {
    entries().insert(
        session_id.to_string(),
        Watch {
            turn_id: turn_id.to_string(),
            created_at,
            from,
        },
    );
    WATCHES.changed.notify_one();
}

/// Watch a turn from its creation (its first step).
pub fn arm(session_id: &str, turn_id: &str, created_at: i64) {
    watch(session_id, turn_id, created_at, created_at);
}

/// Watch every running turn of a full read, from `now` on.
pub fn arm_running(records: &[TurnRecord], now: i64) {
    for record in records.iter().filter(|r| is_running(r.status)) {
        watch(&record.session_id, &record.turn_id, record.created_at, now);
    }
}

/// The turn ended; a newer turn's watch on the same session stays.
pub fn forget(session_id: &str, turn_id: &str) {
    let mut entries = entries();
    if entries
        .get(session_id)
        .is_some_and(|w| w.turn_id == turn_id)
    {
        entries.remove(session_id);
    }
}

/// Move a watch past the check point it just reached.
fn advance(session_id: &str, turn_id: &str, from: i64) {
    if let Some(w) = entries()
        .get_mut(session_id)
        .filter(|w| w.turn_id == turn_id)
    {
        w.from = from;
    }
}

pub fn overdue_event(record: &TurnRecord, now: i64) -> TurnOverdueEvent {
    let pending_calls = record
        .calls
        .iter()
        .filter(|(_, call)| call.state != CallState::Done)
        .map(|(call_id, call)| OverdueCall {
            call_id: call_id.clone(),
            state: match call.state {
                CallState::Pending => "pending",
                _ => "dispatched",
            }
            .to_string(),
            function_id: call.function_id.clone(),
            held_by: call.held_by.clone(),
            child_session_id: call.child_session_id.clone(),
        })
        .collect();
    TurnOverdueEvent {
        session_id: record.session_id.clone(),
        turn_id: record.turn_id.clone(),
        status: match record.status {
            TurnStatus::AwaitingFunctions => "awaiting_functions",
            _ => "running",
        }
        .to_string(),
        depth: record.depth,
        parent_session_id: record.display_parent_session_id.clone(),
        started_at: record.created_at,
        updated_at: record.updated_at,
        age_ms: now - record.created_at,
        step: record.step,
        turn_count: record.turn_count,
        message_preview: record.message_preview.clone(),
        pending_calls,
        timestamp: now,
    }
}

/// Report one watched turn if it still runs and someone observes the event.
async fn report(deps: &Deps, session_id: &str, watch: &Watch, now: i64) {
    if !deps.events.observes_overdue() {
        return;
    }
    let cfg = deps.cfg().await;
    match crate::state::get_turn_unhydrated(&deps.iii, session_id, cfg.session_timeout_ms).await {
        Ok(Some(record)) if record.turn_id == watch.turn_id && is_running(record.status) => {
            deps.events.emit_overdue(&overdue_event(&record, now)).await;
        }
        Ok(_) => forget(session_id, &watch.turn_id),
        Err(e) => tracing::warn!(session_id, error = %e, "could not read an overdue turn"),
    }
}

/// Background driver: sleep until the earliest check point, report the
/// turns that reached theirs, move each to its next one.
pub async fn serve(deps: Arc<Deps>) {
    loop {
        let cfg = deps.cfg().await;
        let (after, every) = (cfg.overdue_after_ms, cfg.overdue_every_ms);
        let now = AgentMessage::now_ms();
        let mut due = Vec::new();
        let mut next: Option<i64> = None;
        entries().retain(|session_id, w| {
            match next_check_point(w.created_at, w.from, after, every) {
                Some(at) if at <= now => due.push((session_id.clone(), w.clone(), at)),
                Some(at) => next = Some(next.map_or(at, |n| n.min(at))),
                // Off: keep watching in case it is turned on.
                None if after == 0 => {}
                None => return false,
            }
            true
        });
        if !due.is_empty() {
            for (session_id, watch, at) in due {
                report(&deps, &session_id, &watch, now).await;
                advance(&session_id, &watch.turn_id, at + 1);
            }
            continue;
        }
        let sleep = async {
            match next {
                Some(at) => {
                    let wait = (at - AgentMessage::now_ms()).max(0) as u64;
                    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                }
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = sleep => {}
            _ = WATCHES.changed.notified() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn check_points_start_after_the_delay_then_repeat() {
        let after = 20 * MIN as u64;
        // Armed at creation: the first check point.
        assert_eq!(next_check_point(0, 0, after, after), Some(20 * MIN));
        // Past one: the next one, never one in between.
        assert_eq!(
            next_check_point(0, 20 * MIN + 1, after, after),
            Some(40 * MIN)
        );
        assert_eq!(next_check_point(0, 39 * MIN, after, after), Some(40 * MIN));
        assert_eq!(next_check_point(0, 40 * MIN, after, after), Some(40 * MIN));
        // Re-armed by a boot read at 65 min: 80, not the 60 it missed.
        assert_eq!(next_check_point(0, 65 * MIN, after, after), Some(80 * MIN));
    }

    #[test]
    fn zero_after_is_off_and_zero_every_reports_once() {
        assert_eq!(next_check_point(0, 0, 0, MIN as u64), None);
        assert_eq!(next_check_point(0, 0, MIN as u64, 0), Some(MIN));
        assert_eq!(next_check_point(0, MIN + 1, MIN as u64, 0), None);
    }

    #[test]
    fn a_newer_turn_keeps_its_watch_when_the_old_one_completes() {
        arm("s-overdue-test", "t1", 0);
        arm("s-overdue-test", "t2", 5);
        forget("s-overdue-test", "t1");
        assert_eq!(
            entries().get("s-overdue-test").map(|w| w.turn_id.clone()),
            Some("t2".into())
        );
        advance("s-overdue-test", "t2", 99);
        assert_eq!(entries().get("s-overdue-test").map(|w| w.from), Some(99));
        forget("s-overdue-test", "t2");
        assert!(!entries().contains_key("s-overdue-test"));
    }
}
