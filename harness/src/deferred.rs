//! Deferred trigger (harness.md § Deferred trigger): a pending call parks the
//! turn; `harness::function::resolve` settles it (delivering a result or
//! releasing a hook-held call) and resumes the parked turn; a cron sweep
//! expires stragglers so a lost child or abandoned approval can never park a
//! turn forever.

use serde_json::{json, Value};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::functions::function_resolve::{FunctionResolveRequest, FunctionResolveResponse};
use crate::ids;
use crate::types::content::ContentBlock;
use crate::types::message::{AgentMessage, FunctionResultMessage, FunctionResultRoleTag};
use crate::types::turn::{CallState, TurnStatus};

/// Settle a pending call and resume the parked turn.
pub async fn resolve(
    deps: &Deps,
    req: FunctionResolveRequest,
) -> Result<FunctionResolveResponse, HarnessError> {
    let _guard = deps.locks.guard(&req.session_id).await;
    let cfg = deps.cfg().await;
    let session = deps.session().await;

    let Some(mut record) =
        crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?
    else {
        return Ok(not_resolved());
    };
    if record.turn_id != req.turn_id || record.status.is_terminal() {
        return Ok(not_resolved());
    }
    let Some(checkpoint) = record.calls.get(&req.function_call_id).cloned() else {
        return Ok(not_resolved());
    };
    if checkpoint.state != CallState::Pending {
        // Already delivered / unknown — duplicate resolves are no-ops.
        return Ok(not_resolved());
    }

    let action = req.action.as_deref().unwrap_or("deliver");
    match action {
        "deliver" => {
            let function_id = checkpoint.function_id.clone().unwrap_or_default();
            let entry_id = ids::function_result_entry_id(&record.turn_id, &req.function_call_id);
            let message = AgentMessage::FunctionResult(FunctionResultMessage {
                role: FunctionResultRoleTag::FunctionResult,
                function_call_id: req.function_call_id.clone(),
                function_id,
                content: req
                    .content
                    .clone()
                    .unwrap_or_else(|| vec![ContentBlock::text("")]),
                details: req.details.clone().unwrap_or(Value::Null),
                is_error: req.is_error.unwrap_or(false),
                timestamp: AgentMessage::now_ms(),
            });
            // Idempotent on the deterministic entry id.
            session
                .append(
                    &record.session_id,
                    &message,
                    Some(&entry_id),
                    None,
                    Some(&json!({ "turn_id": record.turn_id })),
                )
                .await?;

            if let Some(cp) = record.calls.get_mut(&req.function_call_id) {
                cp.state = CallState::Done;
                cp.entry_id = Some(entry_id);
            }

            let turn_resumed = persist_and_maybe_resume(deps, &cfg, &mut record).await?;
            Ok(FunctionResolveResponse {
                resolved: true,
                turn_resumed,
            })
        }
        "execute" => {
            // `execute` releases a `pre_trigger` hook-held call: invoke the
            // target (the holding hook already decided to allow), run the
            // post_trigger chain, append the result, and resume — bypassing
            // the approver's own re-evaluation (harness.md § `function::resolve`).
            if checkpoint.held_by.is_none() {
                return Ok(not_resolved());
            }
            let function_id = checkpoint.function_id.clone().unwrap_or_default();
            let arguments = find_call_arguments(deps, &record, &req.function_call_id)
                .await
                .unwrap_or(Value::Null);
            if let Some(cp) = record.calls.get_mut(&req.function_call_id) {
                cp.state = CallState::Triggered;
            }
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;

            let policy = crate::policy::CompiledPolicy::from(record.options.functions.as_ref());
            let engine = deps.engine().await;
            // Held calls resume outside the inline loop, so use the same
            // invocation chokepoint to keep subscription session injection/owner
            // checks.
            let raw = crate::functions::subscribe::invoke(
                deps,
                &engine,
                &policy,
                &function_id,
                &arguments,
                &record.session_id,
            )
            .await;
            let (data, annotations) = deps
                .hooks
                .run_post_trigger(
                    &record,
                    record.step,
                    &req.function_call_id,
                    &function_id,
                    raw,
                )
                .await;

            let entry_id = ids::function_result_entry_id(&record.turn_id, &req.function_call_id);
            let mut origin = serde_json::Map::new();
            origin.insert("turn_id".into(), json!(record.turn_id));
            for (k, v) in annotations {
                origin.insert(k, v);
            }
            let message = AgentMessage::FunctionResult(FunctionResultMessage {
                role: FunctionResultRoleTag::FunctionResult,
                function_call_id: req.function_call_id.clone(),
                function_id,
                content: data.content,
                details: data.details,
                is_error: data.is_error,
                timestamp: AgentMessage::now_ms(),
            });
            session
                .append(
                    &record.session_id,
                    &message,
                    Some(&entry_id),
                    None,
                    Some(&Value::Object(origin)),
                )
                .await?;
            if let Some(cp) = record.calls.get_mut(&req.function_call_id) {
                cp.state = CallState::Done;
                cp.entry_id = Some(entry_id);
            }
            let turn_resumed = persist_and_maybe_resume(deps, &cfg, &mut record).await?;
            Ok(FunctionResolveResponse {
                resolved: true,
                turn_resumed,
            })
        }
        other => Err(HarnessError::InvalidRequest(format!(
            "unknown resolve action `{other}`"
        ))),
    }
}

/// Persist the record; if no calls remain pending, resume the turn (bump step,
/// re-enqueue) and report `true`.
async fn persist_and_maybe_resume(
    deps: &Deps,
    cfg: &crate::config::WorkerConfig,
    record: &mut crate::types::turn::TurnRecord,
) -> Result<bool, HarnessError> {
    let still_pending = record.calls.values().any(|c| c.state == CallState::Pending);
    if still_pending {
        record.updated_at = AgentMessage::now_ms();
        crate::state::put_turn(&deps.iii, record, cfg.session_timeout_ms).await?;
        return Ok(false);
    }
    record.step += 1;
    record.status = TurnStatus::Running;
    record.updated_at = AgentMessage::now_ms();
    crate::state::put_turn(&deps.iii, record, cfg.session_timeout_ms).await?;
    crate::turn_loop::enqueue_step(&deps.iii, &record.session_id, &record.turn_id, record.step)
        .await?;
    Ok(true)
}

/// Recover a held call's (unwrapped) arguments from the transcript so
/// `execute` can invoke the target. Scans assistant messages for the
/// function_call block with this id.
async fn find_call_arguments(
    deps: &Deps,
    record: &crate::types::turn::TurnRecord,
    call_id: &str,
) -> Option<Value> {
    let session = deps.session().await;
    let entries = session.messages(&record.session_id, false).await.ok()?;
    let expose = record
        .options
        .functions
        .as_ref()
        .map(|f| f.expose)
        .unwrap_or(crate::types::turn::ExposeMode::AgentTrigger);
    for entry in entries.into_iter().rev() {
        if let Some(AgentMessage::Assistant(a)) = entry.message {
            for planned in crate::policy::plan_calls(&a, expose) {
                if planned.id == call_id {
                    return Some(planned.arguments);
                }
            }
        }
    }
    None
}

/// Resolve a parked parent call from a finishing child (harness.md §
/// Sub-agents). `completed` delivers the child's result; `failed`/`cancelled`
/// deliver an `is_error`.
pub async fn resolve_parent(
    deps: &Deps,
    parent: &crate::types::turn::ParentLink,
    status: &str,
    result: Option<&Value>,
    reason: Option<&str>,
) {
    let (content, details, is_error) = if status == "completed" {
        let text = result.map(render_text).unwrap_or_default();
        (
            vec![ContentBlock::text(text)],
            result.cloned().unwrap_or(Value::Null),
            false,
        )
    } else {
        let msg = reason.unwrap_or(status).to_string();
        (
            vec![ContentBlock::text(msg.clone())],
            json!({ "error": status, "message": msg }),
            true,
        )
    };
    let req = FunctionResolveRequest {
        session_id: parent.session_id.clone(),
        turn_id: parent.turn_id.clone(),
        function_call_id: parent.function_call_id.clone(),
        action: Some("deliver".to_string()),
        content: Some(content),
        is_error: Some(is_error),
        details: Some(details),
    };
    if let Err(e) = resolve(deps, req).await {
        tracing::warn!(
            parent_session = %parent.session_id,
            error = %e,
            "resolving parent call from child completion failed"
        );
    }
}

/// Whether a pending checkpoint should be resolved by the expiry sweep.
/// Approval/hook holds (`held_by`) never expire; sub-agent child pendings do.
fn pending_call_expired(
    cp: &crate::types::turn::CallCheckpoint,
    default_timeout_ms: u64,
    now: i64,
) -> bool {
    if cp.state != CallState::Pending {
        return false;
    }
    if cp.held_by.is_some() {
        return false;
    }
    let timeout = cp.pending_timeout_ms.unwrap_or(default_timeout_ms);
    let pending_at = cp.pending_at.unwrap_or(now);
    now.saturating_sub(pending_at) as u64 >= timeout
}

/// Resolve every pending call past its `pending_timeout_ms` with an error.
pub async fn sweep_expired(deps: &Deps) -> Result<u64, HarnessError> {
    let cfg = deps.cfg().await;
    let records = crate::state::list_turns(&deps.iii, cfg.session_timeout_ms).await?;
    let now = AgentMessage::now_ms();

    // Collect expired (session, turn, call) tuples first; resolve re-reads.
    let mut expired: Vec<(String, String, String)> = Vec::new();
    for record in records {
        if record.status != TurnStatus::AwaitingFunctions {
            continue;
        }
        for (call_id, cp) in &record.calls {
            if pending_call_expired(cp, cfg.default_pending_timeout_ms, now) {
                expired.push((
                    record.session_id.clone(),
                    record.turn_id.clone(),
                    call_id.clone(),
                ));
            }
        }
    }

    let mut resolved = 0;
    for (session_id, turn_id, call_id) in expired {
        let req = FunctionResolveRequest {
            session_id,
            turn_id,
            function_call_id: call_id,
            action: Some("deliver".to_string()),
            content: Some(vec![ContentBlock::text(
                "pending call timed out".to_string(),
            )]),
            is_error: Some(true),
            details: Some(json!({ "error": "pending_timeout" })),
        };
        match resolve(deps, req).await {
            Ok(r) if r.resolved => resolved += 1,
            _ => {}
        }
    }
    Ok(resolved)
}

fn not_resolved() -> FunctionResolveResponse {
    FunctionResolveResponse {
        resolved: false,
        turn_resumed: false,
    }
}

fn render_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::turn::{CallCheckpoint, CallState};

    fn cp(
        state: CallState,
        held_by: Option<&str>,
        timeout_ms: Option<u64>,
        pending_at: i64,
    ) -> CallCheckpoint {
        CallCheckpoint {
            state,
            function_id: Some("shell::run".into()),
            entry_id: None,
            child_session_id: if held_by.is_none() && timeout_ms.is_some() {
                Some("child".into())
            } else {
                None
            },
            child_turn_id: if held_by.is_none() && timeout_ms.is_some() {
                Some("t_child".into())
            } else {
                None
            },
            held_by: held_by.map(str::to_string),
            pending_timeout_ms: timeout_ms,
            pending_at: Some(pending_at),
        }
    }

    #[test]
    fn approval_hold_never_expires() {
        let now = 1_000_000;
        let checkpoint = cp(
            CallState::Pending,
            Some("approval::gate"),
            None,
            now - 999_999,
        );
        assert!(!pending_call_expired(&checkpoint, 1_800_000, now));
    }

    #[test]
    fn sub_agent_pending_expires_past_wait_guard() {
        let now = 1_000_000;
        let fresh = cp(CallState::Pending, None, Some(60_000), now - 30_000);
        assert!(!pending_call_expired(&fresh, 1_800_000, now));
        let stale = cp(CallState::Pending, None, Some(60_000), now - 60_000);
        assert!(pending_call_expired(&stale, 1_800_000, now));
    }

    #[test]
    fn sub_agent_pending_uses_default_timeout_when_unset() {
        let now = 1_000_000;
        let default = 1_800_000;
        let stale = cp(CallState::Pending, None, None, now - default as i64);
        assert!(pending_call_expired(&stale, default, now));
    }
}
