//! Wake expiry — a binding that dies without ever firing must WAKE its owner
//! with the news, instead of leaving the session parked forever next to a
//! deadline that already passed.
//!
//! Two paths retire an unfired wake:
//!
//! * its `lifecycle.expires_at` passes — found by the periodic [`sweep`];
//! * a lineage session unregisters it out from under the parked owner
//!   (`engine::unregister_trigger` from a child cleaning up the run).
//!
//! Both end in [`notify_wake_lost`]: a `[notification]` message into the
//! session the wake would have delivered to, plus the same durable
//! `trigger_fired` record every other delivery outcome writes — so the
//! timeline can answer "why did this session un-park with no event?".
//!
//! The sweep DELETES the record before it notifies. The delete is the atomic
//! claim against a concurrent real fire: `claim_fire`'s compare-and-set
//! resolves to `Gone` once the record is missing, so one hand can never tell
//! the owner "nothing is coming" while the other delivers the wake.

use std::sync::Arc;

use serde_json::{json, Value};

use super::Binding;
use crate::deps::Deps;
use crate::subscriptions::fired;
use crate::types::message::AgentMessage;

/// Environment override for the sweep cadence — integration runs shrink it to
/// exercise expiry in seconds. The default trades precision for quiet: a wake
/// deadline is minutes-scale, so a half-minute lag on the notice is noise.
pub const SWEEP_INTERVAL_ENV: &str = "III_HARNESS_EXPIRY_SWEEP_MS";
pub const DEFAULT_SWEEP_INTERVAL_MS: u64 = 30_000;

pub fn sweep_interval_ms() -> u64 {
    std::env::var(SWEEP_INTERVAL_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .unwrap_or(DEFAULT_SWEEP_INTERVAL_MS)
}

/// Periodic expiry sweep, spawned at startup for the worker lifetime.
pub async fn run_loop(deps: Arc<Deps>) {
    let interval = std::time::Duration::from_millis(sweep_interval_ms());
    loop {
        tokio::time::sleep(interval).await;
        sweep(&deps).await;
    }
}

/// One pass: retire every binding whose lifecycle is spent, and tell the
/// owner of any wake that died unfired. Returns how many were retired.
pub async fn sweep(deps: &Deps) -> usize {
    let store = deps.bindings().await;
    let bindings = match store.list().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "expiry sweep skipped: binding store unreadable");
            return 0;
        }
    };
    super::gc::reconcile_orphan_delivery_triggers(deps, &bindings).await;
    let now = AgentMessage::now_ms();
    let mut retired = 0usize;
    for binding in bindings.into_iter().filter(|b| b.is_exhausted(now)) {
        // Delete FIRST, but only while the record still equals the listed
        // snapshot. A racing real fire increments it with CAS; that makes this
        // claim lose instead of deleting the delivered record and emitting a
        // false "expired unfired" notice.
        if !matches!(store.delete_if_unchanged(&binding).await, Ok(true)) {
            continue;
        }
        if let Some(trigger_id) = binding.trigger_id.as_deref() {
            crate::functions::subscribe::unregister_engine_trigger(deps, trigger_id).await;
        }
        retired += 1;
        tracing::info!(
            binding = %binding.id,
            fires = binding.fires,
            "expired binding retired by sweep"
        );
        if is_unfired_wake(&binding) {
            notify_wake_lost(deps, &binding, WakeLost::Expired).await;
        }
    }
    retired
}

/// A wake binding that never delivered — the only shape whose retirement
/// strands a parked session, so the only one that warrants a notification.
pub fn is_unfired_wake(binding: &Binding) -> bool {
    binding.lifecycle.once
        && binding.target.function_id == crate::functions::SEND_ID
        && binding.fires == 0
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
    let (scope, key) = watch_scope_key(binding);
    fired::emit(
        &deps.session().await,
        &binding.owner.session_id,
        &format!("e_trigexpired_{}", binding.id),
        fired::TriggerFired {
            subscription_id: &binding.id,
            trigger_id: binding.trigger_id.as_deref(),
            target: &binding.target.function_id,
            label: None,
            once: binding.lifecycle.once,
            retired: true,
            scope,
            key,
            note: Some(&note),
            fired_at: AgentMessage::now_ms(),
        },
    )
    .await;
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

fn watch_scope_key(binding: &Binding) -> (Option<&str>, Option<&str>) {
    let Some((_, config)) = binding.trigger_watch() else {
        return (None, None);
    };
    (
        config.get("scope").and_then(Value::as_str),
        config.get("key").and_then(Value::as_str),
    )
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

    #[test]
    fn only_a_never_fired_once_wake_warrants_a_notification() {
        assert!(is_unfired_wake(&wake(None)));

        let mut fired = wake(None);
        fired.fires = 1;
        assert!(!is_unfired_wake(&fired), "a delivered wake needs no eulogy");

        let mut standing = wake(None);
        standing.lifecycle.once = false;
        assert!(
            !is_unfired_wake(&standing),
            "a standing notify parks nobody"
        );

        let mut call = wake(None);
        call.target = BindingTarget::new("state::set");
        assert!(!is_unfired_wake(&call), "a call target delivers to no chat");
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
    fn the_expiry_entry_ids_never_collide_with_a_real_fire() {
        // A real fire at ordinal 0 appends `e_fire_sub_1_0` + `e_trigfired_sub_1_0`.
        // The expiry pair must be distinct from both AND from each other, or
        // session-manager's entry-id idempotence swallows one append.
        let wake_id = format!("e_expire_{}", "sub_1");
        let record_id = format!("e_trigexpired_{}", "sub_1");
        assert_ne!(wake_id, record_id);
        assert_ne!(wake_id, "e_fire_sub_1_0");
        assert_ne!(record_id, "e_trigfired_sub_1_0");
    }

    #[test]
    fn sweep_interval_parses_only_positive_ms() {
        // The env override is read directly in sweep_interval_ms; pin the
        // parse rules through the same code path used at startup.
        std::env::remove_var(SWEEP_INTERVAL_ENV);
        assert_eq!(sweep_interval_ms(), DEFAULT_SWEEP_INTERVAL_MS);
        std::env::set_var(SWEEP_INTERVAL_ENV, "500");
        assert_eq!(sweep_interval_ms(), 500);
        std::env::set_var(SWEEP_INTERVAL_ENV, "0");
        assert_eq!(sweep_interval_ms(), DEFAULT_SWEEP_INTERVAL_MS);
        std::env::set_var(SWEEP_INTERVAL_ENV, "nonsense");
        assert_eq!(sweep_interval_ms(), DEFAULT_SWEEP_INTERVAL_MS);
        std::env::remove_var(SWEEP_INTERVAL_ENV);
    }
}
