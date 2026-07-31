//! `harness::stop` — request cancellation of an in-flight turn (harness.md §
//! `harness::stop`). Sets the abort flag the next step observes and aborts an
//! in-flight stream via `router::abort`. The cascade to spawned children
//! layers on with sub-agents.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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

    // Lock-free pre-read: discover the in-flight stream + spawned children and
    // fast-out for a missing/mismatched/terminal turn. This drives the prompt,
    // lock-free part of cancellation (child cascade + router::abort) so
    // generation is interrupted immediately, even while the running step holds
    // the per-session lock across in-flight tool execution. The authoritative
    // abort write happens under the lock below.
    let Some(record) =
        crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?
    else {
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
    // Idempotent: a prior stop already fired the cancel signal, cascaded to
    // children, and requested the router abort — repeat clicks become one read.
    if record.abort {
        return Ok(StopResponse { stopping: true });
    }
    // Pin the turn we observed so the write under the lock can't land on a newer
    // turn that started in between (matters when `turn_id` was omitted).
    let target_turn = record.turn_id.clone();

    // In-process cancel signal, fired lock-free BEFORE anything that awaits:
    // it cuts the in-flight `router.chat` await (backstop when router::abort
    // is a no-op) and is observed between tool executions, where the durable
    // flag write below is blocked on the session lock.
    deps.cancels.fire(&target_turn);

    // Cascade to spawned children BEFORE taking this session's lock
    // (harness.md § Cancellation cascade): each child stop acquires its OWN
    // session lock, so holding the parent lock here could deadlock against a
    // child-side write. Holding at most one session lock at a time keeps
    // cancellation lock-order-free. Stopping a child that already finished is
    // a harmless no-op (fire-and-forget spawns settle Done instantly, so the
    // checkpoint no longer tracks child liveness).
    for child in record.spawned_children() {
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

    // Prompt stream interruption (lock-free): aborting a stale or already
    // finished request_id is a harmless no-op, so the pre-read id is safe to
    // use without the lock.
    if let Some(request_id) = &record.stream_request_id {
        let router = deps.router().await;
        router.abort(request_id).await;
    }

    // Authoritative abort write UNDER the per-session lock (see locks.rs): the
    // running step persists the whole turn record from a stale in-memory copy
    // at several points, so a lock-free write here would be clobbered. Taking
    // the lock — as `harness::function::resolve` and the pending sweep already
    // do — serializes this read-modify-write with the step and closes the race.
    // Re-read inside the lock so the flag is set on the freshest record rather
    // than reverting the step's other updates.
    let _guard = deps.locks.guard(&req.session_id).await;
    let Some(mut record) =
        crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?
    else {
        return Ok(StopResponse { stopping: false });
    };
    if record.turn_id != target_turn {
        // A newer turn started between the pre-read and the lock — don't flag it.
        return Ok(StopResponse { stopping: false });
    }
    if record.status.is_terminal() {
        // The in-process cancel signal and router abort above can finalize the
        // turn before this handler reacquires the session lock. The stop was
        // accepted against the matching non-terminal pre-read; report that
        // acceptance even though the durable abort bit can no longer be set.
        return Ok(StopResponse { stopping: true });
    }
    record.abort = true;
    record.updated_at = crate::types::message::AgentMessage::now_ms();
    crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;

    // "stopping" ack on the existing phase-reason channel (status stays
    // "working" — same semantics as "waiting for <model>"). UNDER the lock and
    // after the terminal re-check: every finalizer emits its own set_status
    // while holding this lock, so a lock-free ack here could land AFTER a
    // concurrent finalize's "done" and leave the session stuck on "working".
    let session = deps.session().await;
    let _ = session
        .set_status(&req.session_id, "working", Some("stopping"))
        .await;

    Ok(StopResponse { stopping: true })
}
