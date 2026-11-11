//! What the worker does about accounts on its own, with or without the app
//! (agent-accounts D12, D14, §3.7): keeps the usage readings fresh while
//! someone watches, reads the global account now and then while the rules
//! are on, and runs the rules.

use std::sync::Arc;
use std::time::Duration;

use super::{Accounts, UNWATCHED_GLOBAL_EVERY_MS};
use crate::collector::Collector;
use crate::events::Monitor;
use crate::model::{now_ms, RulesMode};

/// How often the loop looks.
const TICK: Duration = Duration::from_secs(15);

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
