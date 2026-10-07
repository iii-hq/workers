//! Wake expiry — a binding that dies without ever firing must WAKE its owner
//! with the news, instead of leaving the session parked forever next to a
//! deadline that already passed.
//!
//! Two paths retire an unfired wake:
//!
//! * its `lifecycle.expires_at` passes — the binding's own deadline timer
//!   ([`arm`]) fires at that instant;
//! * a lineage session unregisters it out from under the parked owner
//!   (`engine::unregister_trigger` from a child cleaning up the run).
//!
//! Both end in [`notify_wake_lost`]: a `[notification]` message into the
//! session the wake would have delivered to, plus the same durable
//! `trigger_fired` record every other delivery outcome writes — so the
//! timeline can answer "why did this session un-park with no event?".
//!
//! Deadlines are timers, not a scan. A binding with an `expires_at` gets one
//! one-shot ([`crate::timer::OneShots`], keyed by binding id) armed when it is
//! registered and re-armed from the durable store by every [`sweep`] — run
//! once at boot and again on each engine worker connect/disconnect/announce
//! ([`crate::engine_events`]), which is how the bindings of a harness
//! instance that went away get a timer in the instances that remain. The
//! store cancels the timer whenever it deletes the record: a fire that
//! retires the binding, an unregister, a session teardown.
//!
//! Retirement DELETES the record before it notifies. The delete is the
//! atomic claim against a concurrent real fire: `claim_fire`'s
//! compare-and-set resolves to `Gone` once the record is missing, so one hand
//! can never tell the owner "nothing is coming" while the other delivers the
//! wake.

use std::future::Future;
use std::sync::Arc;

use serde_json::{json, Value};

use super::{Binding, BindingStore};
use crate::deps::Deps;
use crate::subscriptions::fired;
use crate::timer::OneShots;
use crate::types::message::AgentMessage;

/// Store reads at a deadline that fail are retried this many times, on a
/// doubling backoff from [`RETRY_BASE_MS`]. Past that, the next sweep — the
/// state worker coming back is itself an engine worker event — retires it.
const RETRY_ATTEMPTS: u32 = 5;
const RETRY_BASE_MS: i64 = 1_000;
/// Compare-and-delete rounds against concurrent fires before giving up to
/// the next sweep (the same bound the store's own retries use).
const RETIRE_ROUNDS: usize = 8;

/// Boot pass, then one pass per engine worker change. Never on a clock: the
/// deadlines themselves are the binding timers each pass (re-)arms.
pub async fn run(deps: Arc<Deps>) {
    sweep(&deps).await;
    loop {
        deps.kicks.bindings.notified().await;
        sweep(&deps).await;
    }
}

/// One reconciliation pass over the durable store: drop delivery triggers
/// whose record is gone, retry Compose wake recovery, retire every binding
/// whose lifecycle is already spent (an unfired once-wake also wakes its
/// owner), and arm the deadline timer of every other binding that has one.
/// Returns how many were retired.
pub async fn sweep(deps: &Deps) -> usize {
    let store = deps.bindings().await;
    let bindings = match store.list().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "binding pass skipped: binding store unreadable");
            return 0;
        }
    };
    super::gc::reconcile_orphan_delivery_triggers(deps, &bindings).await;
    // Recovery runs independently with bounded concurrency and probe
    // timeouts; an unavailable Compose diagnostic must not hold up expiry.
    // Re-running it from the durable watch on every worker change closes a
    // provider activation that came after the registration-time snapshot,
    // including after a reconnect or a harness restart.
    for binding in &bindings {
        super::compose::schedule(deps, binding);
    }
    let now = AgentMessage::now_ms();
    let mut retired = 0usize;
    for binding in bindings {
        if binding.is_exhausted(now) {
            retired += usize::from(retire_due(deps, &binding.id, now).await);
        } else {
            arm(deps, &binding);
        }
    }
    retired
}

/// Arm `binding`'s deadline timer: at `expires_at` it is retired through
/// [`retire_due`]. A binding without a deadline gets none; one already armed
/// in this process keeps its timer.
pub fn arm(deps: &Deps, binding: &Binding) {
    let worker = deps.clone();
    schedule(&deps.expiry_timers, binding, move |id, due| async move {
        retire_due(&worker, &id, due).await;
    });
}

/// The timer half of [`arm`], over any due action — what the paused-clock
/// tests drive. Returns whether a timer is armed for the binding.
fn schedule<F, Fut>(timers: &OneShots, binding: &Binding, on_due: F) -> bool
where
    F: FnOnce(String, i64) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let Some(due) = binding.lifecycle.expires_at else {
        return false;
    };
    if timers.is_armed(&binding.id) {
        // Deadlines never move once registered; the pending timer stands.
        return true;
    }
    timers.arm(binding.id.clone(), due, on_due(binding.id.clone(), due));
    true
}

/// A binding's deadline `due` passed (by its timer, or a pass found it
/// spent): retire it if the durable record is still spent, re-reading after
/// every lost compare-and-delete — a racing fire that moved the record does
/// not get to strand a standing binding past its deadline. `true` when this
/// caller retired it.
pub async fn retire_due(deps: &Deps, binding_id: &str, due: i64) -> bool {
    retire_due_attempt(deps, binding_id, due, 0).await
}

async fn retire_due_attempt(deps: &Deps, binding_id: &str, due: i64, attempt: u32) -> bool {
    let store = deps.bindings().await;
    for _ in 0..RETIRE_ROUNDS {
        let binding = match store.get(binding_id).await {
            Ok(Some(binding)) => binding,
            // Already retired by a fire, an unregister, or another instance.
            Ok(None) => return false,
            Err(error) => {
                retry_later(deps, binding_id, due, attempt, &error.to_string());
                return false;
            }
        };
        // The timer is the clock: by its measure `due` has passed, even if
        // the wall clock lags a few milliseconds behind the monotonic sleep.
        let now = AgentMessage::now_ms().max(due);
        if !binding.is_exhausted(now) {
            // Not spent at this deadline (a later or no deadline): follow
            // the record instead of retiring early.
            arm(deps, &binding);
            return false;
        }
        if retire_spent(deps, &store, &binding).await {
            return true;
        }
    }
    tracing::warn!(
        binding = %binding_id,
        "expired binding kept moving under retirement; the next binding pass retries"
    );
    false
}

fn retry_later(deps: &Deps, binding_id: &str, due: i64, attempt: u32, error: &str) {
    if attempt >= RETRY_ATTEMPTS {
        tracing::warn!(
            binding = %binding_id,
            error,
            "binding store unreadable at its deadline; the next binding pass retires it"
        );
        return;
    }
    let backoff = RETRY_BASE_MS << attempt;
    tracing::warn!(
        binding = %binding_id,
        error,
        backoff_ms = backoff,
        "binding store unreadable at its deadline; retrying"
    );
    let worker = deps.clone();
    let id = binding_id.to_string();
    deps.expiry_timers
        .arm(binding_id, AgentMessage::now_ms() + backoff, async move {
            retire_due_attempt(&worker, &id, due, attempt + 1).await;
        });
}

/// Retire one spent binding: compare-and-delete FIRST (only while the record
/// still equals this snapshot — a racing fire's CAS makes this claim lose),
/// then unregister its engine trigger, then report an EXPIRY. A binding its
/// delivered fires used up is only cleaned up: the delivery that claimed
/// the last slot already wrote the binding's outcome (or is writing it right
/// now — a pass can land between its claim and its own retirement), so a
/// second, "expired" record would be a false duplicate. `false` when the
/// record moved or the delete failed.
async fn retire_spent(deps: &Deps, store: &BindingStore, binding: &Binding) -> bool {
    if !matches!(store.delete_if_unchanged(binding).await, Ok(true)) {
        return false;
    }
    if let Some(trigger_id) = binding.trigger_id.as_deref() {
        crate::functions::subscribe::unregister_engine_trigger(deps, trigger_id).await;
    }
    if binding.is_spent() {
        tracing::debug!(
            binding = %binding.id,
            fires = binding.fires,
            "spent binding record cleaned up; its delivery owns the outcome"
        );
        return true;
    }
    tracing::info!(
        binding = %binding.id,
        fires = binding.fires,
        "expired binding retired"
    );
    report_expired_retirement(deps, binding).await;
    true
}

/// A wake binding that never delivered — the only shape whose retirement
/// strands a parked session, so the only one that warrants a notification.
pub fn is_unfired_wake(binding: &Binding) -> bool {
    binding.lifecycle.once
        && binding.target.function_id == crate::functions::SEND_ID
        && binding.fires == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpiredRetirementMode {
    NotifyAndRecord,
    RecordOnly,
}

fn expired_retirement_mode(binding: &Binding) -> ExpiredRetirementMode {
    if is_unfired_wake(binding) {
        ExpiredRetirementMode::NotifyAndRecord
    } else {
        ExpiredRetirementMode::RecordOnly
    }
}

/// Why an unfired wake is gone.
pub enum WakeLost<'a> {
    Expired,
    Unregistered { by: &'a str },
}

/// Deliver the news: a `[notification]` into the wake's destination session
/// (so the owner un-parks and gets to run its own fallback — report, tear
/// down, re-arm) and a `trigger_fired` record in the owner's timeline. Both
/// best-effort; the binding is already retired either way.
pub async fn notify_wake_lost(deps: &Deps, binding: &Binding, cause: WakeLost<'_>) {
    let session_id = wake_session(binding);
    let message = AgentMessage::user_text(wake_lost_text(binding, &cause));
    // One expiry per binding ever (the delete claimed it), so the entry ids
    // need no ordinal — and they must never collide with each other or with a
    // real fire's `e_fire_*`/`e_trigfired_*` pair (session-manager dedups on
    // entry ids).
    let wake_entry_id = format!("e_expire_{}", binding.id);
    if let Err(e) = crate::functions::send::inject(
        deps,
        session_id,
        message,
        Some(&wake_entry_id),
        Some(&json!({ "notification": true, "binding": binding.id })),
    )
    .await
    {
        tracing::warn!(
            binding = %binding.id,
            session_id = %session_id,
            error = %e,
            "wake-lost notification failed to deliver"
        );
    }

    let note = match &cause {
        WakeLost::Expired => format!(
            "expired unfired at {}; owner notified",
            binding.lifecycle.expires_at.unwrap_or_default()
        ),
        WakeLost::Unregistered { by } => format!("unregistered by {by} before any fire"),
    };
    let (outcome, retirement_reason) = wake_lost_classification(&cause);
    emit_retirement_record(deps, binding, outcome, retirement_reason, Some(&note)).await;
}

/// Record an expiry after the caller won the binding's retirement CAS. The
/// wake path deliberately delegates to `notify_wake_lost`, which already
/// emits exactly one notification and one retirement record.
pub async fn report_expired_retirement(deps: &Deps, binding: &Binding) {
    match expired_retirement_mode(binding) {
        ExpiredRetirementMode::NotifyAndRecord => {
            notify_wake_lost(deps, binding, WakeLost::Expired).await;
        }
        ExpiredRetirementMode::RecordOnly => {
            let note = format!(
                "expired at {}",
                binding.lifecycle.expires_at.unwrap_or_default()
            );
            emit_retirement_record(
                deps,
                binding,
                fired::TriggerOutcome::Expired,
                fired::RetirementReason::Expired,
                Some(&note),
            )
            .await;
        }
    }
}

async fn emit_retirement_record(
    deps: &Deps,
    binding: &Binding,
    outcome: fired::TriggerOutcome,
    reason: fired::RetirementReason,
    note: Option<&str>,
) {
    fired::emit(
        &deps.session().await,
        &binding.owner.session_id,
        &format!("e_trigexpired_{}", binding.id),
        fired::retirement_record(binding, outcome, reason, note, AgentMessage::now_ms()),
    )
    .await;
}

fn wake_lost_classification(
    cause: &WakeLost<'_>,
) -> (fired::TriggerOutcome, fired::RetirementReason) {
    match cause {
        WakeLost::Expired => (
            fired::TriggerOutcome::Expired,
            fired::RetirementReason::Expired,
        ),
        WakeLost::Unregistered { .. } => (
            fired::TriggerOutcome::Unregistered,
            fired::RetirementReason::Unregistered,
        ),
    }
}

/// The session a wake delivers to — same resolution the live fire path uses.
fn wake_session(binding: &Binding) -> &str {
    binding
        .target
        .payload
        .as_ref()
        .and_then(|p| p.get("session_id"))
        .and_then(Value::as_str)
        .unwrap_or(&binding.owner.session_id)
}

/// The message the woken session reads. It must carry enough to act on
/// without any lookup: what was watched, that zero fires happened, and that
/// NOTHING else will wake the session for it.
fn wake_lost_text(binding: &Binding, cause: &WakeLost<'_>) -> String {
    let watch = match binding.trigger_watch() {
        Some((ttype, config)) => format!(
            "{ttype} {}",
            serde_json::to_string(config).unwrap_or_else(|_| "{}".into())
        ),
        None => "unknown watch".to_string(),
    };
    match cause {
        WakeLost::Expired => format!(
            "[notification] wake expired unfired: {watch} (binding {}) passed its lifecycle \
             deadline (expires_at {}) with 0 fires. Nothing else will wake this session for it — \
             this is the deadline path: decide the fallback now (report what you know, tear \
             down, or re-arm a new watch).",
            binding.id,
            binding.lifecycle.expires_at.unwrap_or_default()
        ),
        WakeLost::Unregistered { by } => format!(
            "[notification] wake removed: {watch} (binding {}) was unregistered by session {by} \
             before it ever fired. Nothing else will wake this session for it.",
            binding.id
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::{BindingTarget, Causation, Lifecycle, OwnerScope};

    fn wake(expires_at: Option<i64>) -> Binding {
        Binding {
            id: "sub_1".into(),
            trigger_id: None,
            owner: OwnerScope {
                session_id: "s_owner".into(),
                root_session_id: None,
            },
            target: BindingTarget::new(crate::functions::SEND_ID),
            conditions: vec![],
            lifecycle: Lifecycle {
                once: true,
                max_fires: None,
                expires_at,
            },
            capability: None,
            causation: Causation::default(),
            dedup_key: Some(json!({
                "trigger_type": "state",
                "config": { "scope": "run", "key": "report_ready" },
            })),
            fires: 0,
            created_at: 0,
        }
    }

    /// The deadline is its own timer: nothing runs while it is ahead — no
    /// sweep, no other wake-up — and the retirement runs AT `expires_at`.
    #[tokio::test(start_paused = true)]
    async fn a_deadline_runs_its_retirement_at_the_due_instant_and_not_before() {
        let timers = OneShots::new();
        let deadline = AgentMessage::now_ms() + 90_000;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let started = tokio::time::Instant::now();
        assert!(schedule(
            &timers,
            &wake(Some(deadline)),
            move |id, due| async move {
                tx.send((id, due, tokio::time::Instant::now())).unwrap();
            }
        ));
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(89_000)).await;
        assert!(
            rx.try_recv().is_err(),
            "a pending deadline runs nothing early"
        );
        let (id, due, fired_at) = rx.recv().await.expect("the deadline must fire");
        assert_eq!(id, "sub_1");
        assert_eq!(due, deadline);
        let waited = (fired_at - started).as_millis();
        assert!(
            (89_990..=90_010).contains(&waited),
            "fired after {waited}ms, not at its 90s deadline"
        );
        assert!(timers.is_empty(), "a fired deadline detaches itself");
    }

    /// A binding deleted before its deadline (a fire that consumed it, an
    /// unregister) has its timer cancelled by the store — the retirement
    /// never runs, however long the clock goes on.
    #[tokio::test(start_paused = true)]
    async fn a_cancelled_deadline_never_runs() {
        let timers = OneShots::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        assert!(schedule(
            &timers,
            &wake(Some(AgentMessage::now_ms() + 60_000)),
            move |id, _| async move {
                tx.send(id).unwrap();
            }
        ));
        tokio::task::yield_now().await;
        assert!(timers.cancel("sub_1"));
        tokio::time::advance(std::time::Duration::from_secs(3_600)).await;
        assert!(rx.recv().await.is_none(), "the cancelled retirement ran");
    }

    #[tokio::test(start_paused = true)]
    async fn only_a_deadline_arms_and_rearming_keeps_the_pending_timer() {
        let timers = OneShots::new();
        assert!(!schedule(&timers, &wake(None), |_, _| async {}));
        assert!(timers.is_empty(), "no deadline, no timer");
        let binding = wake(Some(AgentMessage::now_ms() + 60_000));
        assert!(schedule(&timers, &binding, |_, _| async {}));
        assert!(schedule(&timers, &binding, |_, _| async {
            panic!("a re-arm must not replace the pending deadline");
        }));
        assert_eq!(timers.len(), 1);
    }

    #[test]
    fn only_a_never_fired_once_wake_warrants_a_notification() {
        assert!(is_unfired_wake(&wake(None)));
        assert_eq!(
            expired_retirement_mode(&wake(None)),
            ExpiredRetirementMode::NotifyAndRecord
        );

        let mut fired = wake(None);
        fired.fires = 1;
        assert!(!is_unfired_wake(&fired), "a delivered wake needs no eulogy");
        assert_eq!(
            expired_retirement_mode(&fired),
            ExpiredRetirementMode::RecordOnly
        );

        let mut standing = wake(None);
        standing.lifecycle.once = false;
        assert!(
            !is_unfired_wake(&standing),
            "a standing notify parks nobody"
        );
        assert_eq!(
            expired_retirement_mode(&standing),
            ExpiredRetirementMode::RecordOnly
        );

        let mut call = wake(None);
        call.target = BindingTarget::new("state::set");
        assert!(!is_unfired_wake(&call), "a call target delivers to no chat");
        assert_eq!(
            expired_retirement_mode(&call),
            ExpiredRetirementMode::RecordOnly
        );
    }

    #[test]
    fn recurring_call_expiry_keeps_source_and_count_in_its_record_only_shape() {
        let mut call = wake(Some(1_700_000_000_000));
        call.target = BindingTarget::new("state::set");
        call.lifecycle.once = false;
        call.fires = 4;
        let record = serde_json::to_value(fired::retirement_record(
            &call,
            fired::TriggerOutcome::Expired,
            fired::RetirementReason::Expired,
            Some("expired at 1700000000000"),
            42,
        ))
        .unwrap();
        assert_eq!(record["target"], "state::set");
        assert_eq!(record["trigger_type"], "state");
        assert_eq!(record["config"]["key"], "report_ready");
        assert_eq!(record["fires"], 4);
        assert_eq!(record["outcome"], "expired");
        assert_eq!(record["retirement_reason"], "expired");
        assert_eq!(record["retired"], true);
    }

    #[test]
    fn the_notice_names_the_watch_the_deadline_and_the_finality() {
        let text = wake_lost_text(&wake(Some(1_700_000_000_000)), &WakeLost::Expired);
        assert!(text.starts_with("[notification] "), "got: {text}");
        assert!(text.contains("state"), "got: {text}");
        assert!(text.contains("report_ready"), "got: {text}");
        assert!(text.contains("1700000000000"), "got: {text}");
        assert!(text.contains("0 fires"), "got: {text}");
        assert!(
            text.contains("Nothing else will wake this session"),
            "the finality is the actionable part: {text}"
        );

        let removed = wake_lost_text(&wake(None), &WakeLost::Unregistered { by: "s_child" });
        assert!(
            removed.contains("unregistered by session s_child"),
            "got: {removed}"
        );
        assert!(removed.contains("Nothing else will wake this session"));
    }

    #[test]
    fn expiry_and_manual_unregistration_have_distinct_structured_reasons() {
        assert_eq!(
            wake_lost_classification(&WakeLost::Expired),
            (
                fired::TriggerOutcome::Expired,
                fired::RetirementReason::Expired,
            )
        );
        assert_eq!(
            wake_lost_classification(&WakeLost::Unregistered { by: "console" }),
            (
                fired::TriggerOutcome::Unregistered,
                fired::RetirementReason::Unregistered,
            )
        );
    }

    #[test]
    fn the_expiry_entry_ids_never_collide_with_a_real_fire() {
        // A first real fire appends `e_fire_sub_1_1` + `e_trigfired_sub_1_1`.
        // The expiry pair must be distinct from both AND from each other, or
        // session-manager's entry-id idempotence swallows one append.
        let wake_id = format!("e_expire_{}", "sub_1");
        let record_id = format!("e_trigexpired_{}", "sub_1");
        assert_ne!(wake_id, record_id);
        assert_ne!(wake_id, "e_fire_sub_1_1");
        assert_ne!(record_id, "e_trigfired_sub_1_1");
    }
}
