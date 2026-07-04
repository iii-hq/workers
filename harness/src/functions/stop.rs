//! `harness::stop` — cancel an in-flight turn (harness.md § `harness::stop`).
//! Aborts an in-flight stream via `router::abort`, finalises the turn as
//! cancelled under the session lock (a step that is currently generating
//! finalises itself off the aborted stream instead), then cascades to spawned
//! children. Finalising here — not just flagging — is what unparks a turn
//! whose children died without resolving their calls (MOT-3856).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::types::turn::ParentLink;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct StopRequest {
    pub session_id: String,
    /// Omit to stop the current turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StopResponse {
    pub stopping: bool,
}

pub async fn handle(deps: &Deps, req: StopRequest) -> Result<StopResponse, HarnessError> {
    let cfg = deps.cfg().await;

    // Lock-free pre-read: discover the in-flight stream + live children and
    // fast-out for a missing/mismatched/terminal turn.
    let Some(record) =
        crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?
    else {
        // No turn record (e.g. loop state lost across a worker restart), but
        // the session may still be wedged at "working" — clear it so the
        // console recovers without a manual `session::set-status idle`
        // (MOT-3856). Idempotent on an already-idle session.
        let session = deps.session().await;
        let _ = session.set_status(&req.session_id, "idle", None).await;
        return Ok(StopResponse { stopping: false });
    };
    if let Some(tid) = &req.turn_id {
        if &record.turn_id != tid {
            return Ok(StopResponse { stopping: false });
        }
    }
    if record.status.is_terminal() {
        return Ok(StopResponse { stopping: false });
    }
    // Pin the turn we observed so the finalise under the lock can't land on a
    // newer turn that started in between (matters when `turn_id` was omitted).
    let target_turn = record.turn_id.clone();
    let mut children = record.live_children();

    // Prompt stream interruption (lock-free): a generating step holds the
    // session lock, so this must fire BEFORE the guard below — the aborted
    // stream ends the step quickly, releasing the lock. Aborting a stale or
    // already finished request_id is a harmless no-op.
    if let Some(request_id) = &record.stream_request_id {
        let router = deps.router().await;
        router.abort(request_id).await;
    }

    // Finalise UNDER the per-session lock (see locks.rs): holding the guard
    // proves no step is executing (run_step guards the whole step, generation
    // included), so nothing is left to observe a mere abort flag — a turn
    // parked on dead children would stay wedged forever (MOT-3856). A queued
    // step then sees the terminal status and skips, and a late child resolve
    // is a no-op. The guard drops before the child cascade below: each child
    // stop finalises under its OWN lock and resolves upward into this session,
    // so holding this lock across the cascade would deadlock.
    {
        let _guard = deps.locks.guard(&req.session_id).await;
        let Some(mut record) =
            crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?
        else {
            return Ok(StopResponse { stopping: false });
        };
        if record.turn_id != target_turn {
            // A newer turn started between the pre-read and the lock — don't
            // touch it, but the pinned turn's children still need stopping.
            cascade(deps, &children).await;
            return Ok(StopResponse { stopping: false });
        }
        let finalized_by_step = record.status.is_terminal();
        if !finalized_by_step {
            // Freshest child set: a step that ran between the pre-read and the
            // lock may have spawned more children.
            children = record.live_children();
            record.abort = true;
            let session = deps.session().await;
            crate::turn_loop::finalize_cancelled(deps, &session, &mut record, "cancelled").await?;
        }
        // else: the step finalised between the pre-read and the lock (possibly
        // via the router::abort above) — the turn is settled, but its children
        // were only observed by the pre-read, so still cascade.
    }

    // Cascade to non-terminal spawned children (harness.md § Cancellation
    // cascade), one session lock at a time. Their parent-call resolves land on
    // an already-terminal turn and no-op.
    cascade(deps, &children).await;

    Ok(StopResponse { stopping: true })
}

async fn cascade(deps: &Deps, children: &[ParentLink]) {
    for child in children {
        Box::pin(handle(
            deps,
            StopRequest {
                session_id: child.session_id.clone(),
                turn_id: Some(child.turn_id.clone()),
            },
        ))
        .await
        .ok();
    }
}
