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
    if let Some(want) = &req.turn_id {
        if crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms)
            .await?
            .is_none_or(|r| &r.turn_id != want)
        {
            return Ok(StopResponse { stopping: false });
        }
    }
    let tree = super::session_tree::collect(deps, &req.session_id).await?;
    if !tree.complete && !tree.sessions.is_empty() {
        return Err(HarnessError::InvalidRequest(
            "incomplete cancellation tree".into(),
        ));
    }
    // Signal all descendants before waiting for a single busy session lock.
    for node in &tree.sessions {
        if let Some(record) =
            crate::state::get_turn(&deps.iii, &node.session_id, cfg.session_timeout_ms).await?
        {
            if !record.status.is_terminal() {
                deps.cancels.fire(&record.turn_id);
            }
        }
    }
    let mut stopping = false;
    let mut errors = Vec::new();
    for node in tree.sessions {
        match stop_one(
            deps,
            StopRequest {
                session_id: node.session_id,
                turn_id: None,
            },
        )
        .await
        {
            Ok(response) => stopping |= response.stopping,
            Err(error) => errors.push(error.to_string()),
        }
    }
    if !errors.is_empty() {
        return Err(HarnessError::Dependency(errors.join("; ")));
    }
    Ok(StopResponse { stopping })
}

pub(crate) async fn stop_one(deps: &Deps, req: StopRequest) -> Result<StopResponse, HarnessError> {
    let cfg = deps.cfg().await;

    // Lock-free pre-read: discover the in-flight stream + spawned children and
    // fast-out for a missing/mismatched/terminal turn. This drives the prompt,
    // lock-free part of cancellation (child cascade + router::abort) so
    // generation is interrupted immediately, even while the running step holds
    // the per-session lock across in-flight tool execution. The authoritative
    // abort write happens under the lock below.
    //
    // Nothing to stop (no record, or a terminal one): the store may still say
    // `working` — a terminal projection lost while the session-manager was
    // down, or a state store that dropped the record. Re-derive it in the
    // background so the click repairs what it found instead of silently
    // doing nothing.
    let Some(record) =
        crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?
    else {
        crate::session_status::spawn_reconcile(deps, &req.session_id);
        return Ok(StopResponse { stopping: false });
    };
    if let Some(tid) = &req.turn_id {
        if &record.turn_id != tid {
            return Ok(StopResponse { stopping: false });
        }
    }
    if record.status.is_terminal() {
        crate::session_status::spawn_project(deps, record);
        return Ok(StopResponse { stopping: false });
    }
    // Pin the turn we observed so the write under the lock can't land on a newer
    // turn that started in between (matters when `turn_id` was omitted).
    let target_turn = record.turn_id.clone();

    // In-process cancel signal, fired lock-free BEFORE anything that awaits:
    // it cuts the in-flight `router.chat` await (backstop when router::abort
    // is a no-op) and is observed between tool executions, where the durable
    // flag write below is blocked on the session lock.
    deps.cancels.fire(&target_turn);

    // Prompt stream interruption (lock-free): aborting a stale or already
    // finished request_id is a harmless no-op, so the pre-read id is safe to
    // use without the lock.
    if let Some(request_id) = &record.stream_request_id {
        deps.iii
            .trigger(iii_sdk::protocol::TriggerRequest {
                function_id: "router::abort".into(),
                payload: serde_json::json!({"request_id": request_id}),
                action: None,
                timeout_ms: Some(cfg.session_timeout_ms),
            })
            .await
            .map_err(|e| HarnessError::Dependency(format!("router::abort {request_id}: {e}")))?;
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

    // A parked approval has no queued step to observe abort. Finalize it
    // under the same lock, but refuse unknown external pending work.
    if record.status == crate::types::turn::TurnStatus::AwaitingFunctions {
        if record.calls.values().any(|call| {
            call.state == crate::types::turn::CallState::Pending
                && call.held_by.is_none()
                && call.child_session_id.is_none()
        }) {
            return Err(HarnessError::Dependency(
                "pending external tool has no confirmed cancellation".into(),
            ));
        }
        crate::turn_loop::finalize_cancelled(
            deps,
            &deps.session().await,
            &mut record,
            "cancelled by user",
        )
        .await?;
        deps.deletion_changed.notify_waiters();
        return Ok(StopResponse { stopping: true });
    }

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
