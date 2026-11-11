//! What the worker does about accounts on its own, with or without the app
//! (agent-accounts D12, D14, §3.7): keeps the usage readings fresh while
//! someone watches, reads the global account now and then while the rules
//! are on and another signed-in account could take over (O1), runs the
//! rules, and moves the panes whose move waited for the end of their turn.

use std::sync::Arc;
use std::time::Duration;

use super::{Accounts, UNWATCHED_GLOBAL_EVERY_MS};
use crate::collector::Collector;
use crate::events::Monitor;
use crate::model::{now_ms, RulesMode};

/// How often the loop looks.
const TICK: Duration = Duration::from_secs(15);
/// How often panes waiting for their turn to end are looked at.
const PENDING_TICK: Duration = Duration::from_secs(2);

/// Whether the live feed has an observer now.
pub type Observed = Arc<dyn Fn() -> bool + Send + Sync>;

pub fn spawn(collector: Arc<Collector>, monitor: Arc<Monitor>, observed: Observed) {
    tokio::spawn(async move {
        let accounts = collector.accounts().clone();
        let relink = accounts.clone();
        let _ = tokio::task::spawn_blocking(move || relink.relink_all()).await;
        // With accounts.json, one look at who is signed in, so the rules
        // know whether another account could take over (O1). Without it,
        // nothing is read until someone watches.
        if accounts.managed() {
            accounts.load_now().await;
        }
        loop {
            tick(&collector, &monitor, &observed).await;
            tokio::time::sleep(TICK).await;
        }
    });
}

/// Moves the panes whose move waits for their turn to end (§3.7).
pub fn spawn_pending(collector: Arc<Collector>, monitor: Arc<Monitor>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(PENDING_TICK).await;
            pending_tick(&collector, &monitor).await;
        }
    });
}

/// Whether the pane whose move waits may be ready: its Claude Code process
/// is gone or says `idle` (a small file read, no collection).
fn may_be_ready(claude_pid: Option<u32>) -> bool {
    let Some(pid) = claude_pid else {
        return true;
    };
    // A hint only: the move itself matches the record exactly.
    match crate::activity::claude_record_now(pid, false) {
        None => true,
        Some(record) => record
            .status
            .as_deref()
            .is_some_and(super::panes::is_idle_status),
    }
}

/// One look at the panes whose move waits for their turn (§3.7, M8): their
/// Claude Code records are read, and a fresh collection is made only when
/// one of them may be ready (or after a start, to find the marks a restart
/// left). An idle one moves; one that runs where it should (or no longer
/// runs) loses the mark; one whose move can't happen any more loses it too,
/// and its pane says why (`move_error`).
pub async fn pending_tick(collector: &Arc<Collector>, monitor: &Arc<Monitor>) {
    let accounts: &Arc<Accounts> = collector.accounts();
    let discover = accounts.take_pending();
    let waiting = accounts.waiting();
    if !discover && waiting.is_empty() {
        return;
    }
    // A pane being moved (its Claude Code still ending late, say) is not
    // ready: nothing is collected for it until that move is over.
    let waiting: Vec<(String, Option<u32>)> = waiting
        .into_iter()
        .filter(|(pane, _)| !accounts.moving.is_busy(pane))
        .collect();
    let ready = discover
        || tokio::task::spawn_blocking(move || waiting.iter().any(|(_, pid)| may_be_ready(*pid)))
            .await
            .unwrap_or(true);
    if !ready {
        return;
    }
    let Ok(tmux) = collector.tmux().await else {
        accounts.note_pending();
        return;
    };
    // A fresh look notes again the panes that still carry the mark.
    let panes = collector.claude_panes(true).await;
    for (pane, _) in accounts.waiting() {
        if panes
            .get(&pane)
            .is_none_or(|state| state.marks.pending.is_none())
        {
            accounts.forget_waiting(&pane);
        }
    }
    if panes.values().all(|pane| pane.marks.pending.is_none()) {
        return;
    }
    // Sign-ins are read only for the moves someone asked for that can
    // happen now (idle panes), and only those moves' accounts.
    let registry = accounts.registry();
    let mut targets: Vec<String> = panes
        .values()
        .filter(|pane| pane.marks.pending.is_some() && pane.idle())
        .filter(|pane| !accounts.moving.is_busy(&pane.pane_id))
        .filter_map(|pane| super::moves::pending_target(pane, &registry))
        .collect();
    targets.sort();
    targets.dedup();
    accounts.load_accounts_now(targets).await;
    let mut changed = false;
    let mut ids: Vec<&String> = panes.keys().collect();
    ids.sort();
    for id in ids {
        let pane = &panes[id];
        let Some(plan) =
            super::moves::pending_plan(pane, &registry, |to| accounts.is_signed_in(to))
        else {
            continue;
        };
        let outcome = match plan {
            Ok(plan) => super::moves::apply(&tmux, accounts, pane, &plan).await,
            Err(error) => Err(error),
        };
        match outcome {
            Ok(state) => {
                tracing::info!(pane = %id, state, "pending account move");
                changed = true;
            }
            // Being moved: it keeps waiting, looked at once that is over.
            Err(error) if error == super::moves::BUSY => accounts.note_waiting(id, pane.claude_pid),
            Err(error) => {
                tracing::warn!(pane = %id, %error, "pending account move dropped");
                accounts.note_move_error(id, &error);
                changed = true;
                let mark = pane
                    .marks
                    .account
                    .clone()
                    .unwrap_or_else(|| super::GLOBAL.into());
                match super::moves::set_marks(&tmux, id, &mark, pane.info.skip_permissions, None)
                    .await
                {
                    Ok(()) => accounts.forget_waiting(id),
                    // The mark is still there: the pane keeps waiting, and
                    // the next look drops it again.
                    Err(mark_error) => {
                        tracing::warn!(pane = %id, error = %mark_error, "pending mark not cleared");
                        accounts.note_waiting(id, pane.claude_pid);
                    }
                }
            }
        }
    }
    if changed {
        monitor.poke();
    }
}

async fn tick(collector: &Arc<Collector>, monitor: &Arc<Monitor>, observed: &Observed) {
    let accounts: &Arc<Accounts> = collector.accounts();
    if observed() {
        accounts.mark_watched();
    }
    // A tmux server started since (or a hand edit) gets the global account
    // back in its environment (D3).
    if let Ok(tmux) = collector.tmux().await {
        if let Err(error) = super::tmux_env::reconcile(&tmux, accounts).await {
            tracing::debug!(%error, "tmux global environment not checked");
        }
    }
    let now = now_ms();
    // Nothing local is read (sign-in files, the Keychain, Codex) unless
    // someone watches or another signed-in account could take over (O1).
    if !accounts.may_load(now) {
        return;
    }
    accounts.load_now().await;
    let registry = accounts.registry();
    let changed = if accounts.watched(now) {
        accounts.refresh(None, false).await
    } else if registry.rules.mode != RulesMode::Off && accounts.may_read_unwatched() {
        // Nobody watches: only the global account, now and then.
        let global = registry.global.clone();
        let stale = accounts
            .reading_age_ms(&global, now)
            .is_none_or(|age| age >= UNWATCHED_GLOBAL_EVERY_MS);
        stale && accounts.refresh(Some(&[global]), false).await
    } else {
        false
    };
    let switched = run_rules(collector).await;
    if changed || switched {
        monitor.poke();
    }
}

/// Mode `switch` and `back_after_reset` (D14): make another account global
/// when the rules call for it, and leave `last_switch` for the app. Returns
/// whether anything changed (a reading or the global account).
pub async fn run_rules(collector: &Arc<Collector>) -> bool {
    let accounts: &Arc<Accounts> = collector.accounts();
    if !accounts.managed() {
        return false;
    }
    // What the rules decide on: a switch is dropped when the registry is
    // written meanwhile (M7), even back to the same values.
    let (registry, generation) = accounts.registry_at();
    // Only mode `switch` acts by itself (a suggestion is the snapshot's).
    if registry.rules.mode != RulesMode::Switch {
        return false;
    }
    // Nobody watching and no other signed-in account: nothing to switch to,
    // and nothing is read (O1).
    if !accounts.may_load(now_ms()) {
        return false;
    }
    accounts.load_now().await;
    let mut snapshot = accounts.snapshot(&std::collections::HashMap::new(), 0);
    // The global account runs low: the others are read (when due) to pick
    // the target (D12).
    let global_low = snapshot
        .claude
        .accounts
        .iter()
        .find(|account| account.id == registry.global)
        .and_then(|global| super::rules::low_reason(global.usage.as_ref(), &registry.rules))
        .is_some();
    let read = global_low && accounts.refresh(None, false).await;
    if read {
        snapshot = accounts.snapshot(&std::collections::HashMap::new(), 0);
    }
    let now = now_ms();
    let Some(decision) = super::rules::decide(&registry, &snapshot.claude.accounts, now) else {
        return read;
    };
    let Ok(tmux) = collector.tmux().await else {
        return read;
    };
    let panes = collector.claude_panes(true).await;
    let mode = if registry.rules.move_idle {
        super::moves::SwitchMode::IdleNowWorkingAfter
    } else {
        super::moves::SwitchMode::NewOnly
    };
    let from = registry.global.clone();
    tracing::info!(%from, to = %decision.to, reason = %decision.reason, "switching the global account");
    let by = super::moves::SwitchBy::Rules {
        generation,
        record: crate::model::AccountSwitch {
            from,
            to: decision.to.clone(),
            reason: Some(decision.reason.clone()),
            auto: true,
            moved: Vec::new(),
            after_turn: Vec::new(),
            at_ms: now,
            back_at_ms: decision.back_at_ms,
        },
    };
    match super::moves::switch_global(&tmux, accounts, &panes, &decision.to, mode, by).await {
        Ok(_) => true,
        Err(error) if error == super::moves::RULES_STALE => {
            tracing::info!(
                "the registry changed while the rules decided; looked at again next time"
            );
            read
        }
        Err(error) => {
            tracing::warn!(%error, "the global account was not switched");
            read
        }
    }
}
