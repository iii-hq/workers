//! `harness::sweep-pending` — the cron-bound expiry sweep for parked pending
//! calls (harness.md § Deferred trigger). Resolves calls past their
//! `pending_timeout_ms` with an error so a lost child or abandoned approval
//! can never park a turn forever. Full resolution lands with deferred trigger.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;

pub const SWEEP_PENDING_ID: &str = "harness::sweep-pending";
pub const SWEEP_PENDING_DESC: &str =
    "Internal cron sweep: resolve pending function calls past their timeout and re-enqueue the \
     step of running turns left without one, so a turn never wedges. Not called directly.";

/// Cron event payload (ignored — the sweep scans all turn records). A struct
/// keeps the request schema concrete.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct SweepEvent {
    #[serde(default)]
    pub scheduled_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SweepResponse {
    pub ok: bool,
    /// Number of expired pending calls resolved this sweep.
    pub resolved: u64,
    /// Number of orphaned running turns whose step was re-enqueued.
    #[serde(default)]
    pub redriven: u64,
}

pub async fn handle(deps: &Deps, _event: SweepEvent) -> Result<SweepResponse, HarnessError> {
    let resolved = crate::deferred::sweep_expired(deps).await?;
    // A failed orphan scan must not hide the pending-call result.
    let redriven = match crate::inflight::redrive_orphans(deps).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, "orphaned-turn redrive sweep failed");
            0
        }
    };
    Ok(SweepResponse {
        ok: true,
        resolved,
        redriven,
    })
}
