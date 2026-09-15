//! The coarse `session::status` (`working` / `done` / `error` / `idle`) is a
//! projection of the durable turn record, never a second source of truth.
//! The loop writes the live phases itself (`working` + "preparing context",
//! "waiting for <model>", "stopping"); everything terminal is derived here,
//! and every path that observes a terminal or missing record re-derives it:
//! the finalizers, `harness::stop`, `harness::status`, and a boot sweep.
//!
//! WHY: the two stores never reconciled. A finalizer wrote the terminal
//! record, then fired `session::set-status` once, fire-and-forget; when the
//! session-manager was down at that instant (a restart mid-turn), the
//! session stayed `working` forever and Stop fast-outed on the terminal
//! record without touching it.

use std::time::Duration;

use crate::clients::SessionClient;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::types::turn::{TurnRecord, TurnStatus};

/// The session status a turn status projects to, or `None` while the turn is
/// live (the loop owns those phases).
pub fn projection_for(
    status: TurnStatus,
    result_error: Option<&str>,
) -> Option<(&'static str, Option<&str>)> {
    match status {
        TurnStatus::Completed => Some(("done", None)),
        TurnStatus::Cancelled => Some(("done", Some("stopped"))),
        TurnStatus::Failed => Some(("error", result_error)),
        TurnStatus::Running | TurnStatus::AwaitingFunctions => None,
    }
}

pub fn projection(record: &TurnRecord) -> Option<(&'static str, Option<&str>)> {
    projection_for(record.status, record.result_error.as_deref())
}

const WRITE_ATTEMPTS: u32 = 2;
const WRITE_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Write a terminal record's projection. `session::set-status` is idempotent
/// (an unchanged status is a no-op), so one retry is safe; a write lost past
/// that is repaired by the next [`repair`] (stop, status, boot).
pub async fn project(session: &SessionClient, record: &TurnRecord) {
    let Some((status, reason)) = projection(record) else {
        return;
    };
    write(session, &record.session_id, status, reason).await;
}

async fn write(session: &SessionClient, session_id: &str, status: &str, reason: Option<&str>) {
    for attempt in 1..=WRITE_ATTEMPTS {
        match session.set_status(session_id, status, reason).await {
            Ok(()) => return,
            Err(error) if attempt < WRITE_ATTEMPTS => {
                tracing::warn!(%session_id, %error, attempt, "session status projection failed; retrying");
                tokio::time::sleep(WRITE_RETRY_DELAY).await;
            }
            Err(error) => {
                tracing::warn!(%session_id, %error, "session status projection lost; the next stop/status/boot reconcile repairs it");
            }
        }
    }
}

/// Re-derive the status of a session the store reports `working`. Returns
/// whether a write was issued: a terminal record projects itself, a missing
/// record (a state store that lost it) means no turn is known — `idle` — and
/// a live record is left alone.
pub async fn repair(deps: &Deps, session_id: &str) -> Result<bool, HarnessError> {
    let cfg = deps.cfg().await;
    let session = deps.session().await;
    match crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms).await? {
        Some(record) if record.status.is_terminal() => {
            project(&session, &record).await;
            Ok(true)
        }
        // ponytail: a live record whose queue job is gone (an enqueue wedge) is
        // left alone; a queue-aware check is the upgrade path.
        Some(_) => Ok(false),
        None => {
            write(&session, session_id, "idle", None).await;
            Ok(true)
        }
    }
}

/// [`repair`] guarded by the store: only a session still marked `working` is
/// touched, so a plain read never flips a finished session that has no record.
pub async fn reconcile(deps: &Deps, session_id: &str) -> Result<bool, HarnessError> {
    let session = deps.session().await;
    if session.status(session_id).await?.as_deref() != Some("working") {
        return Ok(false);
    }
    repair(deps, session_id).await
}

/// Background projection from a request handler: the reply never waits on
/// the session-manager; the `status-changed` event carries the repair.
pub fn spawn_project(deps: &Deps, record: TurnRecord) {
    let deps = deps.clone();
    tokio::spawn(async move {
        let session = deps.session().await;
        project(&session, &record).await;
    });
}

/// Background [`reconcile`] from a request handler.
pub fn spawn_reconcile(deps: &Deps, session_id: &str) {
    let deps = deps.clone();
    let session_id = session_id.to_string();
    tokio::spawn(async move {
        if let Err(error) = reconcile(&deps, &session_id).await {
            tracing::warn!(%session_id, %error, "session status reconcile failed");
        }
    });
}

const SWEEP_ATTEMPTS: u32 = 6;
const SWEEP_RETRY_DELAY: Duration = Duration::from_secs(5);

/// Boot sweep: every session the store still reports `working` is checked
/// against its record. The session-manager may come up after the harness, so
/// the listing is retried for a while, then given up with a warning.
pub async fn sweep(deps: &Deps) {
    let session = deps.session().await;
    let mut ids = None;
    for attempt in 1..=SWEEP_ATTEMPTS {
        match session.working_session_ids().await {
            Ok(found) => {
                ids = Some(found);
                break;
            }
            Err(error) if attempt < SWEEP_ATTEMPTS => {
                tracing::debug!(%error, attempt, "session status sweep: session-manager not ready; retrying");
                tokio::time::sleep(SWEEP_RETRY_DELAY).await;
            }
            Err(error) => {
                tracing::warn!(%error, "session status sweep skipped: session-manager unreachable");
            }
        }
    }
    let Some(ids) = ids else {
        return;
    };
    let mut repaired = 0usize;
    for session_id in &ids {
        match repair(deps, session_id).await {
            Ok(true) => repaired += 1,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%session_id, %error, "session status sweep: repair failed");
            }
        }
    }
    if !ids.is_empty() {
        tracing::info!(working = ids.len(), repaired, "session status sweep");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_statuses_project_and_live_ones_do_not() {
        assert_eq!(
            projection_for(TurnStatus::Completed, Some("ignored")),
            Some(("done", None))
        );
        assert_eq!(
            projection_for(TurnStatus::Cancelled, None),
            Some(("done", Some("stopped")))
        );
        assert_eq!(
            projection_for(TurnStatus::Failed, Some("boom")),
            Some(("error", Some("boom")))
        );
        assert_eq!(projection_for(TurnStatus::Running, None), None);
        assert_eq!(projection_for(TurnStatus::AwaitingFunctions, None), None);
    }
}
