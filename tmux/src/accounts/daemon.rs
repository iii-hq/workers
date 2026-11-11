//! What the worker does about accounts on its own, with or without the app
//! (agent-accounts D12, D14, §3.7): keeps the usage readings fresh while
//! someone watches, reads the global account now and then while the rules
//! are on, runs the rules, and moves the panes whose move waited for the
//! end of their turn.

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
        let _ = tokio::task::spawn_blocking(move || accounts.relink_all()).await;
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

/// One look at the panes with `@iii_claude_pending`: an idle one moves, one
/// that runs where it should (or no longer runs) loses the mark, one still
/// working waits (its target follows a global account that changed again).
pub async fn pending_tick(collector: &Arc<Collector>, monitor: &Arc<Monitor>) {
    let accounts: &Arc<Accounts> = collector.accounts();
    if !accounts.take_pending() {
        return;
    }
    let Ok(tmux) = collector.tmux().await else {
        accounts.note_pending();
        return;
    };
    // A fresh look notes again the panes that still carry the mark.
    let panes = collector.claude_panes(true).await;
    let registry = accounts.registry();
    let mut changed = false;
    let mut ids: Vec<&String> = panes.keys().collect();
    ids.sort();
    for id in ids {
        let pane = &panes[id];
        let Some(plan) = super::moves::pending_plan(pane, &registry) else {
            continue;
        };
        match super::moves::apply(&tmux, accounts, pane, &plan).await {
            Ok(state) => {
                tracing::info!(pane = %id, state, "pending account move");
                changed = true;
            }
            Err(error) if error == super::moves::BUSY => accounts.note_pending(),
            Err(error) => {
                tracing::warn!(pane = %id, %error, "pending account move dropped");
                let mark = pane
                    .marks
                    .account
                    .clone()
                    .unwrap_or_else(|| super::GLOBAL.into());
                let _ = super::moves::set_marks(&tmux, id, &mark, pane.info.skip_permissions, None)
                    .await;
                changed = true;
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
    let registry = accounts.registry();
    let changed = if accounts.watched(now) {
        accounts.refresh(None, false).await
    } else if registry.rules.mode != RulesMode::Off {
        let global = registry.global.clone();
        let stale = accounts
            .reading_age_ms(&global, now)
            .is_none_or(|age| age >= UNWATCHED_GLOBAL_EVERY_MS);
        stale && accounts.refresh(Some(&[global]), false).await
    } else {
        false
    };
    if changed {
        monitor.poke();
    }
}
