//! Rollover, exemption pruning, and the per-budget mutex map. Mirrors
//! roster/workers/llm-budget/src/worker.ts (`maybeRollOver`,
//! `pruneExemptions`, `withBudgetLock`).

use std::collections::HashMap;
use std::sync::Arc;

use iii_sdk::III;
use tokio::sync::{Mutex, OnceCell};
use tracing::info;

use crate::periods::next_period_start;
use crate::store::{save_spend_log, Budget, SpendLogEntry};

static LOCKS: OnceCell<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceCell::const_new();

async fn lock_map() -> &'static Mutex<HashMap<String, Arc<Mutex<()>>>> {
    LOCKS
        .get_or_init(|| async { Mutex::new(HashMap::new()) })
        .await
}

/// Single-process serializer for a budget's load → mutate → save cycle.
/// Matches Roster's `withBudgetLock`. Horizontal scale would need
/// state-backed locks; out of scope here (engine has no CAS yet).
pub async fn with_budget_lock<F, Fut, T>(budget_id: &str, fut_factory: F) -> anyhow::Result<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let m = lock_map().await;
    let entry = {
        let mut g = m.lock().await;
        g.entry(budget_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = entry.lock().await;
    let out = fut_factory().await;
    // Best-effort cleanup: drop empty entries when nobody else is queued.
    if Arc::strong_count(&entry) == 2 {
        let mut g = m.lock().await;
        if let Some(v) = g.get(budget_id) {
            if Arc::strong_count(v) == 2 {
                g.remove(budget_id);
            }
        }
    }
    out
}

/// Roll forward the budget if `ts` has crossed the period reset boundary.
///
/// Archives every closed period to spend log. Returns the new budget; the
/// caller is responsible for persisting it (avoids double-writes).
pub async fn maybe_roll_over(iii: &III, b: Budget, ts: i64) -> anyhow::Result<Budget> {
    if ts < b.period_resets_at {
        return Ok(b);
    }
    let mut period_start_at = b.period_start_at;
    let mut resets_at = b.period_resets_at;
    let mut spent = b.spent_usd;
    let mut archived = 0u32;
    while ts >= resets_at {
        save_spend_log(
            iii,
            &b.id,
            period_start_at,
            &SpendLogEntry {
                budget_id: b.id.clone(),
                period_start: period_start_at,
                period_end: resets_at,
                spent_usd: spent,
                records_count: 0,
            },
        )
        .await?;
        archived += 1;
        period_start_at = resets_at;
        resets_at = next_period_start(b.period, period_start_at);
        spent = 0.0;
    }
    if archived > 0 {
        info!(budget_id = %b.id, archived_count = archived, "budget rolled over");
    }
    let mut alerts = b.alerts;
    for a in &mut alerts {
        a.last_fired_period_start = None;
    }
    Ok(Budget {
        period_start_at,
        period_resets_at: resets_at,
        spent_usd: spent,
        alerts,
        updated_at: ts,
        ..b
    })
}

pub fn prune_exemptions(b: Budget, ts: i64) -> Budget {
    let live: Vec<_> = b
        .exemptions
        .iter()
        .filter(|e| e.expires_at > ts)
        .cloned()
        .collect();
    if live.len() == b.exemptions.len() {
        return b;
    }
    Budget {
        exemptions: live,
        updated_at: ts,
        ..b
    }
}
