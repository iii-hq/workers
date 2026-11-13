//! What the worker does about accounts on its own, with or without the app
//! (agent-accounts D12, D14, §3.7, G4, G6, G7, G11): at start, finishes a
//! swap a stopped worker left and brings a registry of 0.3.0 to the slot;
//! keeps the usage readings fresh while someone watches, reads the global
//! account now and then while the rules are on and another signed-in
//! account could take over (O1), runs the rules, notices a sign-in by hand
//! on the slot, finishes a switch that waited for windows, and moves the
//! panes whose move waited for the end of their turn.

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
        start(&collector, &accounts).await;
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

/// Once at start: a swap a stopped worker left is finished (G7), a
/// registry of 0.3.0 is brought to the slot with tmux (G11), shared items
/// are linked again, and a sign-in by hand while the worker was away is
/// taken in (G6).
pub async fn start(collector: &Arc<Collector>, accounts: &Arc<Accounts>) {
    // Nothing switches, recovers or moves a pane meanwhile.
    let _gate = accounts.swap_gate.lock().await;
    match accounts.recover().await {
        Ok(Some(journal)) => {
            tracing::info!(from = %journal.from, to = %journal.to, "a stopped switch was finished")
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(error = %error.sentence(), "a stopped switch was not finished")
        }
    }
    let upgrade = accounts.clone();
    if tokio::task::spawn_blocking(move || upgrade.needs_upgrade())
        .await
        .unwrap_or(false)
    {
        // tmux first: should the worker stop in between, the next start
        // does it again (the registry still says 0.3.0).
        // A listing that failed re-pins nothing: the registry stays at
        // 0.3.0 for the next start to do it all again.
        let migrated = match collector.tmux().await {
            Ok(tmux) => match collector.claude_panes_checked().await {
                Ok(panes) => {
                    super::switch::migrate_tmux(&tmux, accounts, &panes).await;
                    true
                }
                Err(error) => {
                    tracing::warn!(%error, "panes not listed; accounts brought to the global slot on the next start");
                    false
                }
            },
            // No tmux: no pane to re-pin.
            Err(_) => true,
        };
        if migrated {
            let upgrade = accounts.clone();
            match tokio::task::spawn_blocking(move || upgrade.upgrade()).await {
                Ok(Ok(())) => tracing::info!("accounts brought to the global slot"),
                Ok(Err(error)) => {
                    tracing::warn!(%error, "accounts not brought to the global slot")
                }
                Err(error) => tracing::warn!(%error, "accounts not brought to the global slot"),
            }
        }
    }
    let relink = accounts.clone();
    let _ = tokio::task::spawn_blocking(move || {
        relink.relink_all();
        // Locks the worker moved aside and could not remove (it stopped
        // in between, or someone's could not go back): gone once stale.
        if let Some(stores) = relink.stores() {
            let folders: Vec<super::slot::Place> = relink
                .registry()
                .accounts
                .iter()
                .map(|entry| super::slot::Place::Folder(entry.id.clone()))
                .collect();
            stores.sweep_aside(&folders);
        }
    })
    .await;
    accounts.watch_slot().await;
}

/// Moves the panes whose move waits for their turn to end (§3.7), finishes
/// a switch that waits (G4) and notices a sign-in by hand (G6).
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
    // One at a time with the switches and the moves someone asked for: a
    // look in the middle of one would see the stores half-way. Busy now,
    // the next tick looks.
    let Ok(_gate) = accounts.swap_gate.try_lock() else {
        return;
    };
    // A swap whose write failed after the new account's chain reached the
    // slot (G7): settled as soon as the stores allow it.
    if accounts.has_journal() {
        match accounts.recover().await {
            Ok(Some(journal)) => {
                tracing::info!(from = %journal.from, to = %journal.to, "a switch left half-way was finished");
                monitor.poke();
            }
            Ok(None) => monitor.poke(),
            Err(error) => {
                // Nothing moves until it is: the registry may not name the
                // account the slot holds yet.
                tracing::debug!(error = %error.sentence(), "a switch left half-way is not settled yet");
                return;
            }
        }
    }
    // A sign-in by hand on the slot (G6): a stat, a read when it changed.
    if accounts.watch_slot().await {
        monitor.poke();
    }
    let release = accounts.login_release_due();
    let global_due = global_may_be_ready(accounts).await;
    let discover = accounts.take_pending();
    let waiting = accounts.waiting();
    if !discover && waiting.is_empty() && !global_due && !release {
        return;
    }
    // A pane being moved (its Claude Code still ending late, say) is not
    // ready: nothing is collected for it until that move is over.
    let waiting: Vec<(String, Option<u32>)> = waiting
        .into_iter()
        .filter(|(pane, _)| !accounts.moving.is_busy(pane))
        .collect();
    let ready = discover
        || global_due
        || release
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
    // A fresh look notes again the panes that still carry the mark. One
    // that failed says nothing about them: they keep waiting.
    let mut panes = match collector.claude_panes_checked().await {
        Ok(panes) => panes,
        Err(error) => {
            tracing::debug!(%error, "pending moves not looked at");
            if discover {
                accounts.note_pending();
            }
            return;
        }
    };
    let mut changed = false;
    let released = super::switch::after_login(&tmux, accounts, &panes).await;
    if !released.is_empty() {
        panes = super::switch::as_followers(&panes, &released);
        changed = true;
    }
    if global_due && super::switch::finish_pending(&tmux, accounts, &panes).await {
        changed = true;
    }
    for (pane, _) in accounts.waiting() {
        if panes
            .get(&pane)
            .is_none_or(|state| state.marks.pending.is_none())
        {
            accounts.forget_waiting(&pane);
        }
    }
    if panes.values().all(|pane| pane.marks.pending.is_none()) {
        if changed {
            monitor.poke();
        }
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
    let mut ids: Vec<&String> = panes.keys().collect();
    ids.sort();
    for id in ids {
        let pane = &panes[id];
        if pane.marks.pending.is_none() {
            continue;
        }
        // Each move before this one can take seconds: someone may have
        // cancelled or changed this pane's move, or switched the global
        // account, meanwhile. A request no longer as it was is left to the
        // next round, which sees it as it is now.
        match super::moves::marks_now(&tmux, id).await {
            Ok(now) if now.account == pane.marks.account && now.pending == pane.marks.pending => {}
            _ => continue,
        }
        let registry = accounts.registry();
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
    let mut snapshot = accounts.snapshot(&Default::default());
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
        snapshot = accounts.snapshot(&Default::default());
    }
    let now = now_ms();
    let Some(decision) = super::rules::decide(&registry, &snapshot.claude.accounts, now) else {
        return read;
    };
    // The panes as they are under the gate, which the switch holds on,
    // from a look that listed them all: an empty one from a failed look
    // would take every folder for free (G4).
    let _gate = accounts.swap_gate.lock().await;
    let Ok(tmux) = collector.tmux().await else {
        return read;
    };
    let panes = match collector.claude_panes_checked().await {
        Ok(panes) => panes,
        Err(error) => {
            tracing::debug!(%error, "panes not listed; the rules switch on a later look");
            return read;
        }
    };
    // Never an account with Claude Code working on its folder, or one that
    // can't be restarted: the next one instead (G12).
    let unready = super::switch::unready_targets(&panes);
    let decision = if unready.contains(&decision.to) {
        let ready: Vec<crate::model::ClaudeAccount> = snapshot
            .claude
            .accounts
            .iter()
            .filter(|account| account.id == registry.global || !unready.contains(&account.id))
            .cloned()
            .collect();
        match super::rules::decide(&registry, &ready, now) {
            Some(decision) => decision,
            None => return read,
        }
    } else {
        decision
    };
    let from = registry.global.clone();
    tracing::info!(%from, to = %decision.to, reason = %decision.reason, "switching the global account");
    let by = super::switch::By::Rules {
        generation,
        record: crate::model::AccountSwitch {
            from,
            to: decision.to.clone(),
            reason: Some(decision.reason.clone()),
            auto: true,
            following: 0,
            restarted: Vec::new(),
            after_turn: Vec::new(),
            at_ms: now,
            back_at_ms: decision.back_at_ms,
        },
    };
    match super::switch::switch_locked(&tmux, accounts, &panes, &decision.to, by).await {
        Ok(_) => true,
        Err(super::switch::SwitchError::Stale) => {
            tracing::info!(
                "the registry changed while the rules decided; looked at again next time"
            );
            read
        }
        // A sign-in from the app runs where the switch would go: the rules
        // switch once it is over.
        Err(super::switch::SwitchError::SigningIn(account)) => {
            tracing::debug!(%account, "a sign-in is under way; the rules switch after it");
            read
        }
        Err(error) => {
            tracing::warn!(error = %error.sentence(), "the global account was not switched");
            read
        }
    }
}

/// Whether a switch waits (G4) and may be ready: some Claude Code it waits
/// for is gone or idle, or nothing is known of them (a worker started
/// since). A small file read per pane, no collection.
async fn global_may_be_ready(accounts: &Arc<Accounts>) -> bool {
    let Some(pending) = accounts.registry().pending_global else {
        return false;
    };
    // After a failure that may pass, not before its next try is due.
    if !accounts.pending_try_due(&pending) {
        return false;
    }
    let Some(pids) = accounts.global_waiting() else {
        return true;
    };
    tokio::task::spawn_blocking(move || pids.iter().all(|pid| may_be_ready(*pid)))
        .await
        .unwrap_or(true)
}
