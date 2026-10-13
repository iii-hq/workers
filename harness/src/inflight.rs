//! Orphaned-turn recovery.
//!
//! A turn record can be `Running` with no `harness::turn` step on the queue:
//! the enqueue that should carry its current step failed (queue worker down,
//! namespace-unaware queue provider, engine restart) after the record was
//! persisted, or the worker holding the step went away. Nothing would ever
//! run that step again, so the session stays "working" forever and even
//! `harness::stop` cannot finish it — stop only sets the abort bit the next
//! step observes.
//!
//! [`InflightSteps`] tracks the sessions whose step is executing in this
//! process right now. [`redrive_orphans`] re-enqueues the current step of
//! every `Running` turn that has not moved for [`ORPHAN_REDRIVE_AFTER_MS`] and
//! is not executing here. Re-enqueueing is safe because steps are
//! at-least-once: the queue is FIFO per `session_id` and `generate_step` acks
//! any delivery whose `(turn_id, step)` is no longer current, so a duplicate
//! of a step that was only delayed is dropped.
//!
//! What runs a pass — never a clock ([`run`]):
//!
//! * boot, once the queue and this worker's registrations are back;
//! * every engine worker connect/disconnect/announce
//!   ([`crate::engine_events`]) — the queue returning, a harness instance or
//!   provider going away, an engine restart reconnecting everyone;
//! * the daily pending sweep (`harness::sweep-pending`).
//!
//! A pass cannot judge a turn that moved within the window, so each `Running`
//! turn it finds not yet stale becomes a [`Suspects`] entry with ONE
//! follow-up at its own staleness deadline (`updated_at` + the window): still
//! unmoved then, it is re-driven; moved, it is alive and forgotten. A failed
//! enqueue ([`suspect_unenqueued`]) is the same kind of known-due follow-up.

use std::collections::{BTreeMap, HashMap};
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

/// The redrive's view of the turn scope (key -> last seen `Running`), held
/// for a whole read so the event-driven passes and the cron sweep's (and the
/// sweep's full read, [`read_all_turns`]) serialize; see
/// [`crate::state::read_changed_turns`].
static TURN_VIEW: tokio::sync::Mutex<BTreeMap<String, bool>> =
    tokio::sync::Mutex::const_new(BTreeMap::new());

/// Every turn record ([`crate::state::list_turns`]), with the redrive's view
/// rebuilt from them: the pending sweep's full read doubles as the view's
/// refresh, so a record rewritten behind this process's writes (a state-store
/// rollback, a console edit, another harness process) is redriven after the
/// next sweep, not only after a restart. Writes that land during the read
/// stay marked for the next pass.
pub async fn read_all_turns(deps: &Deps) -> Result<crate::state::TurnListing, HarnessError> {
    let cfg = deps.cfg().await;
    let mut view = TURN_VIEW.lock().await;
    let listing = crate::state::list_turns(&deps.iii, cfg.session_timeout_ms).await?;
    *view = listing
        .records
        .iter()
        .map(|r| (r.session_id.clone(), r.status == TurnStatus::Running))
        .collect();
    Ok(listing)
}

/// Re-enqueue the current step of every orphaned `Running` turn. Returns the
/// number of steps re-enqueued. Reads only the turn records that changed
/// since the last pass ([`crate::state::read_changed_turns`]).
pub async fn redrive_orphans(deps: &Deps) -> Result<u64, HarnessError> {
    Ok(redrive_pass(deps).await?.redriven)
}

/// What one pass did, and which `Running` turns it could not judge yet.
#[derive(Debug, Default)]
pub struct RedrivePass {
    pub redriven: u64,
    /// `Running`, not executing here, and moved within the window:
    /// `(session_id, turn_id, step, updated_at)`.
    pub too_recent: Vec<(String, String, u64, i64)>,
}

async fn redrive_pass(deps: &Deps) -> Result<RedrivePass, HarnessError> {
    let cfg = deps.cfg().await;
    let records = {
        let mut view = TURN_VIEW.lock().await;
        crate::state::read_changed_turns(&deps.iii, &mut view, cfg.session_timeout_ms).await?
    };
    let now = AgentMessage::now_ms();
    let mut pass = RedrivePass::default();
    for record in records {
        let executing_here = deps.inflight.contains(&record.session_id);
        if !is_orphan_candidate(&record, executing_here, now) {
            if record.status == TurnStatus::Running && !executing_here {
                pass.too_recent.push((
                    record.session_id.clone(),
                    record.turn_id.clone(),
                    record.step,
                    record.updated_at,
                ));
            }
            continue;
        }
        match redrive_candidate(deps, record, now).await {
            Ok(true) => pass.redriven += 1,
            Ok(false) => {}
            // One failure must not strand the orphans after it.
            Err(_) => {}
        }
    }
    Ok(pass)
}

/// Re-drive one orphan candidate: re-check under the session lock against
/// the freshest record so a step that just advanced or finished is not
/// redriven, then re-enqueue and restart its window. `Ok(false)` when the
/// fresh record is no longer an orphan. Unhydrated: the check reads no prompt
/// text, and a turn whose prompt body is gone must still be redriven so its
/// step fails it instead of leaving it Running.
async fn redrive_candidate(
    deps: &Deps,
    mut record: TurnRecord,
    now: i64,
) -> Result<bool, HarnessError> {
    let cfg = deps.cfg().await;
    let _guard = deps.locks.guard(&record.session_id).await;
    match crate::state::get_turn_unhydrated(&deps.iii, &record.session_id, cfg.session_timeout_ms)
        .await
    {
        Ok(Some(fresh))
            if fresh.turn_id == record.turn_id
                && fresh.step == record.step
                && is_orphan_candidate(&fresh, deps.inflight.contains(&fresh.session_id), now) =>
        {
            record = fresh;
        }
        Err(e) => {
            tracing::warn!(
                session_id = %record.session_id,
                error = %e,
                "could not re-read an orphan candidate; skipped this pass"
            );
            return Err(e);
        }
        Ok(_) => return Ok(false),
    }
    match redrive_if_idle(deps, &record).await {
        Ok(true) => {
            // Restart the window so the next pass does not re-enqueue a step
            // that is merely waiting its turn on the queue.
            record.updated_at = now;
            let _ = crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await;
            tracing::warn!(
                session_id = %record.session_id,
                turn_id = %record.turn_id,
                step = record.step,
                "re-enqueued the step of an orphaned running turn"
            );
            Ok(true)
        }
        Ok(false) => Ok(false),
        Err(e) => {
            tracing::warn!(
                session_id = %record.session_id,
                turn_id = %record.turn_id,
                error = %e,
                "could not re-enqueue an orphaned running turn"
            );
            Err(e)
        }
    }
}

/// A `Running` turn awaiting one follow-up check at a known instant.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Suspect {
    turn_id: String,
    step: u64,
    /// The record's `updated_at` when suspected; any later write means the
    /// turn moved, i.e. it is alive. `None` for a failed enqueue, whose
    /// record clock alone decides.
    seen_updated_at: Option<i64>,
    due: i64,
    /// Failed follow-ups so far (store unreadable, enqueue failed).
    attempts: u32,
}

/// Failed follow-ups back off from one window, doubling, this many times;
/// past that the suspect is left to the next engine worker event (the queue
/// or state worker coming back is one) or the daily sweep.
const SUSPECT_ATTEMPTS: u32 = 5;

/// Turns awaiting a follow-up at their staleness deadline. Only ever fed by
/// an event-driven pass or a failed enqueue, and every entry resolves (moved,
/// re-driven, or out of attempts) — there is no standing re-check.
#[derive(Default)]
pub struct Suspects {
    entries: Mutex<BTreeMap<String, Suspect>>,
    changed: tokio::sync::Notify,
}

/// What a follow-up concluded about one suspect.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Gone, finished, moved on, moved at all, or executing here.
    Alive,
    /// Unmoved but inside its window until this instant.
    NotYet(i64),
    Orphan,
}

fn verdict(
    record: Option<&TurnRecord>,
    suspect: &Suspect,
    executing_here: bool,
    now: i64,
) -> Verdict {
    let Some(record) = record else {
        return Verdict::Alive;
    };
    if record.status != TurnStatus::Running
        || record.turn_id != suspect.turn_id
        || record.step != suspect.step
        || executing_here
        || suspect
            .seen_updated_at
            .is_some_and(|seen| record.updated_at != seen)
    {
        return Verdict::Alive;
    }
    if is_orphan_candidate(record, executing_here, now) {
        Verdict::Orphan
    } else {
        Verdict::NotYet(record.updated_at + ORPHAN_REDRIVE_AFTER_MS as i64)
    }
}

impl Suspects {
    /// Watch `session_id`'s `(turn_id, step)` until `due`. An existing watch
    /// on the same step keeps its attempts; a newer sighting replaces its
    /// deadline, except that a known-unenqueued step (`seen_updated_at:
    /// None`) stays judged by its record clock alone — a write that does not
    /// advance the step (a stop request) does not put a step on the queue.
    pub fn watch(
        &self,
        session_id: &str,
        turn_id: &str,
        step: u64,
        seen_updated_at: Option<i64>,
        due: i64,
    ) {
        {
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            match entries.get_mut(session_id) {
                Some(s) if s.turn_id == turn_id && s.step == step => {
                    if s.seen_updated_at.is_some() {
                        s.seen_updated_at = seen_updated_at;
                        s.due = due;
                    } else {
                        s.due = s.due.min(due);
                    }
                }
                _ => {
                    entries.insert(
                        session_id.to_string(),
                        Suspect {
                            turn_id: turn_id.to_string(),
                            step,
                            seen_updated_at,
                            due,
                            attempts: 0,
                        },
                    );
                }
            }
        }
        self.changed.notify_one();
    }

    fn next_due(&self) -> Option<i64> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.values().map(|s| s.due).min()
    }

    fn due(&self, now: i64) -> Vec<(String, Suspect)> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .iter()
            .filter(|(_, s)| s.due <= now)
            .map(|(id, s)| (id.clone(), s.clone()))
            .collect()
    }

    /// Apply `f` to the entry only while it still watches `suspect`'s step.
    fn update(&self, session_id: &str, suspect: &Suspect, f: impl FnOnce(&mut Option<Suspect>)) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut slot = entries.remove(session_id);
        if slot
            .as_ref()
            .is_some_and(|s| s.turn_id == suspect.turn_id && s.step == suspect.step)
        {
            f(&mut slot);
        }
        if let Some(entry) = slot {
            entries.insert(session_id.to_string(), entry);
        }
    }

    fn forget(&self, session_id: &str, suspect: &Suspect) {
        self.update(session_id, suspect, |slot| *slot = None);
    }

    fn reschedule(&self, session_id: &str, suspect: &Suspect, due: i64) {
        self.update(session_id, suspect, |slot| {
            if let Some(entry) = slot {
                entry.due = due;
            }
        });
    }

    /// Back off after a failed follow-up; drops the entry once out of
    /// attempts.
    fn back_off(&self, session_id: &str, suspect: &Suspect, now: i64) {
        self.update(session_id, suspect, |slot| {
            if let Some(entry) = slot {
                entry.attempts += 1;
                if entry.attempts > SUSPECT_ATTEMPTS {
                    tracing::warn!(
                        session_id,
                        turn_id = %entry.turn_id,
                        "orphan follow-up out of attempts; the next worker event or sweep retries"
                    );
                    *slot = None;
                    return;
                }
                entry.due = now + ((ORPHAN_REDRIVE_AFTER_MS as i64) << (entry.attempts - 1));
            }
        });
    }
}

/// The process's suspects: fed by [`suspect_unenqueued`] (which has no
/// [`Deps`]) as well as by the passes, drained by [`run`].
static SUSPECTS: std::sync::LazyLock<Suspects> = std::sync::LazyLock::new(Suspects::default);

/// A `Running` turn's step could not be enqueued: unless something else moves
/// it, nothing will ever run it. Follow up once its window has passed — the
/// same staleness bar every orphan meets before it is re-driven.
pub fn suspect_unenqueued(session_id: &str, turn_id: &str, step: u64) {
    SUSPECTS.watch(
        session_id,
        turn_id,
        step,
        None,
        AgentMessage::now_ms() + ORPHAN_REDRIVE_AFTER_MS as i64,
    );
}

/// One follow-up per due suspect.
async fn follow_up(deps: &Deps, suspects: &Suspects) {
    let now = AgentMessage::now_ms();
    let timeout_ms = deps.cfg().await.session_timeout_ms;
    for (session_id, suspect) in suspects.due(now) {
        let record = match crate::state::get_turn_unhydrated(&deps.iii, &session_id, timeout_ms)
            .await
        {
            Ok(record) => record,
            Err(e) => {
                tracing::warn!(session_id = %session_id, error = %e, "orphan follow-up read failed");
                suspects.back_off(&session_id, &suspect, now);
                continue;
            }
        };
        match verdict(
            record.as_ref(),
            &suspect,
            deps.inflight.contains(&session_id),
            now,
        ) {
            Verdict::Alive => suspects.forget(&session_id, &suspect),
            Verdict::NotYet(due) => suspects.reschedule(&session_id, &suspect, due),
            Verdict::Orphan => {
                let record = record.expect("an orphan verdict has a record");
                match redrive_candidate(deps, record, now).await {
                    Ok(_) => suspects.forget(&session_id, &suspect),
                    Err(_) => suspects.back_off(&session_id, &suspect, now),
                }
            }
        }
    }
}

/// An event-driven pass: re-drive what is stale now, follow up on the rest
/// at their own deadlines.
pub async fn event_pass(deps: &Deps) -> u64 {
    let pass = match redrive_pass(deps).await {
        Ok(pass) => pass,
        Err(e) => {
            tracing::warn!(error = %e, "orphaned-turn redrive pass failed");
            return 0;
        }
    };
    watch_too_recent(&SUSPECTS, &pass);
    pass.redriven
}

fn watch_too_recent(suspects: &Suspects, pass: &RedrivePass) {
    for (session_id, turn_id, step, updated_at) in &pass.too_recent {
        suspects.watch(
            session_id,
            turn_id,
            *step,
            Some(*updated_at),
            updated_at + ORPHAN_REDRIVE_AFTER_MS as i64,
        );
    }
}

/// Delay before the first orphan pass after boot, so the queue worker and the
/// harness's own registrations are back before anything is re-enqueued.
const BOOT_DELAY_MS: u64 = 30_000;

/// Background driver: one orphan pass shortly after boot (turns stranded by
/// the outage that preceded a restart), then [`serve`].
///
/// Boot starts with one full read ([`read_all_turns`]): it seeds the
/// redrive's view, so later passes read only running turns, and the records
/// feed one compaction ([`crate::turn_compaction`]) after that pass.
pub async fn run(deps: std::sync::Arc<Deps>) {
    tokio::time::sleep(std::time::Duration::from_millis(BOOT_DELAY_MS)).await;
    let boot_listing = read_all_turns(&deps)
        .await
        .inspect_err(|e| tracing::warn!(error = %e, "boot read of the turn records failed"))
        .ok();
    event_pass(&deps).await;
    if let Some(listing) = boot_listing {
        let report = crate::turn_compaction::compact(&deps, &listing, AgentMessage::now_ms()).await;
        tracing::info!(
            converted = report.converted,
            prompts_collected = report.prompts_collected,
            "turn records compacted after boot"
        );
    }
    serve(deps).await;
}

/// The event loop: a pass per engine worker change (`deps.kicks.turns`), a
/// follow-up whenever the earliest suspect falls due. Nothing else wakes it.
pub async fn serve(deps: std::sync::Arc<Deps>) {
    loop {
        let next_due = SUSPECTS.next_due();
        let deadline = async {
            match next_due {
                Some(due) => {
                    let wait = (due - AgentMessage::now_ms()).max(0) as u64;
                    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                }
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = deps.kicks.turns.notified() => {
                event_pass(&deps).await;
            }
            // A new or earlier suspect: recompute the deadline.
            _ = SUSPECTS.changed.notified() => {}
            _ = deadline => follow_up(&deps, &SUSPECTS).await,
        }
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

    fn suspect(seen: Option<i64>) -> Suspect {
        Suspect {
            turn_id: "t_1".into(),
            step: 0,
            seen_updated_at: seen,
            due: 0,
            attempts: 0,
        }
    }

    #[test]
    fn a_follow_up_redrives_only_an_unmoved_stale_step() {
        let window = ORPHAN_REDRIVE_AFTER_MS as i64;
        let seen = 1_000;
        let stale_now = seen + window;
        let running = record("running", seen);
        assert_eq!(
            verdict(Some(&running), &suspect(Some(seen)), false, stale_now),
            Verdict::Orphan
        );
        // Inside the window: one more follow-up, at the record's own deadline.
        assert_eq!(
            verdict(Some(&running), &suspect(Some(seen)), false, stale_now - 1),
            Verdict::NotYet(stale_now)
        );
        // Any movement, a finished turn, or a step running here is alive.
        let moved = record("running", seen + 5);
        assert_eq!(
            verdict(Some(&moved), &suspect(Some(seen)), false, stale_now + 5),
            Verdict::Alive
        );
        assert_eq!(
            verdict(
                Some(&record("completed", seen)),
                &suspect(Some(seen)),
                false,
                stale_now
            ),
            Verdict::Alive
        );
        assert_eq!(
            verdict(Some(&running), &suspect(Some(seen)), true, stale_now),
            Verdict::Alive
        );
        assert_eq!(
            verdict(None, &suspect(Some(seen)), false, stale_now),
            Verdict::Alive
        );
        let mut next_step = suspect(Some(seen));
        next_step.step = 1;
        assert_eq!(
            verdict(Some(&running), &next_step, false, stale_now),
            Verdict::Alive
        );
        // A failed enqueue is judged by the record clock alone: a write that
        // does not advance the step (a stop request) queues nothing.
        let stopped = record("running", seen + 5);
        assert_eq!(
            verdict(Some(&stopped), &suspect(None), false, seen + 5 + window),
            Verdict::Orphan
        );
    }

    #[test]
    fn suspects_resolve_and_never_become_a_standing_recheck() {
        let suspects = Suspects::default();
        assert_eq!(suspects.next_due(), None);
        suspects.watch("s_1", "t_1", 0, Some(1), 500);
        suspects.watch("s_2", "t_1", 0, None, 300);
        assert_eq!(suspects.next_due(), Some(300));
        let due = suspects.due(400);
        assert_eq!(due.len(), 1);
        let (session, s2) = &due[0];
        assert_eq!(session, "s_2");
        // A newer sighting of a known-unenqueued step keeps the earlier due.
        suspects.watch("s_2", "t_1", 0, Some(9), 900);
        assert_eq!(suspects.due(400).len(), 1);
        suspects.forget(session, s2);
        assert_eq!(suspects.next_due(), Some(500));
        // A newer sighting of an event suspect replaces its deadline.
        suspects.watch("s_1", "t_1", 0, Some(2), 700);
        assert_eq!(suspects.next_due(), Some(700));
        // A forget for a step the entry no longer watches is a no-op.
        suspects.forget("s_1", &suspect(Some(1)).clone_with_step(3));
        assert_eq!(suspects.next_due(), Some(700));
        // Failed follow-ups back off by doubling windows, then give up.
        let (_, s1) = suspects.due(700).remove(0);
        let window = ORPHAN_REDRIVE_AFTER_MS as i64;
        for attempt in 0..SUSPECT_ATTEMPTS {
            suspects.back_off("s_1", &s1, 1_000);
            assert_eq!(suspects.next_due(), Some(1_000 + (window << attempt)));
        }
        suspects.back_off("s_1", &s1, 1_000);
        assert_eq!(
            suspects.next_due(),
            None,
            "out of attempts, the entry is dropped"
        );
    }

    impl Suspect {
        fn clone_with_step(&self, step: u64) -> Self {
            Self {
                step,
                ..self.clone()
            }
        }
    }

    #[test]
    fn a_pass_follows_up_on_the_running_turns_it_could_not_judge_yet() {
        let suspects = Suspects::default();
        let pass = RedrivePass {
            redriven: 0,
            too_recent: vec![("s_1".into(), "t_1".into(), 2, 10_000)],
        };
        watch_too_recent(&suspects, &pass);
        assert_eq!(
            suspects.next_due(),
            Some(10_000 + ORPHAN_REDRIVE_AFTER_MS as i64)
        );
    }
}
