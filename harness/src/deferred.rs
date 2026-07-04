//! Deferred trigger (harness.md § Deferred trigger): a pending call parks the
//! turn; `harness::function::resolve` settles it (delivering a result or
//! releasing a hook-held call) and resumes the parked turn; a cron sweep
//! settles calls whose child turn died without resolving and expires
//! stragglers past their wait guard, so a lost child can never park a turn
//! forever (MOT-3856). Hook holds (`held_by`) never expire.

use std::collections::BTreeSet;

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
            // The release path runs OUTSIDE the turn loop, so re-apply the
            // filesystem scope stamp the loop would have added: without it an
            // approved shell/coder call runs un-scoped (the session's picked
            // directory becomes unreachable) and a model-supplied fs_scope
            // recovered from the transcript would survive un-stripped.
            let session_grants = crate::filesystem_grants::roots(
                &deps.iii,
                &record.session_id,
                cfg.session_timeout_ms,
            )
            .await?;
            let trusted_roots = union_roots(
                session_grants,
                req.fs_scope
                    .as_ref()
                    .map(|s| s.grants.clone())
                    .unwrap_or_default(),
            );
            let filesystem_root = record.options.filesystem_root().map(str::to_string);
            let arguments = crate::filesystem_scope::inject(
                &function_id,
                arguments,
                filesystem_root.as_deref(),
                &trusted_roots,
            );
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
            let post_outcome = deps
                .hooks
                .run_post_trigger(
                    &record,
                    record.step,
                    &req.function_call_id,
                    &function_id,
                    &arguments,
                    raw,
                )
                .await;
            let (data, annotations) = match post_outcome {
                crate::hooks::runner::PostTriggerOutcome::Result {
                    result,
                    annotations,
                } => (result, annotations),
                crate::hooks::runner::PostTriggerOutcome::Hold {
                    held_by,
                    annotations: _,
                } => {
                    if let Some(cp) = record.calls.get_mut(&req.function_call_id) {
                        cp.state = CallState::Pending;
                        cp.held_by = Some(held_by);
                        cp.pending_at = Some(AgentMessage::now_ms());
                    }
                    record.status = TurnStatus::AwaitingFunctions;
                    record.updated_at = AgentMessage::now_ms();
                    crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
                    return Ok(FunctionResolveResponse {
                        resolved: true,
                        turn_resumed: false,
                    });
                }
            };

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
/// Sub-agents). `completed` delivers the child's result; any other status
/// delivers an `is_error` with that status as the error code. Returns whether
/// the call was actually resolved (a terminal/mismatched parent no-ops).
pub async fn resolve_parent(
    deps: &Deps,
    parent: &crate::types::turn::ParentLink,
    status: &str,
    result: Option<&Value>,
    reason: Option<&str>,
) -> bool {
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
        fs_scope: None,
        content: Some(content),
        is_error: Some(is_error),
        details: Some(details),
    };
    match resolve(deps, req).await {
        Ok(r) => r.resolved,
        Err(e) => {
            tracing::warn!(
                parent_session = %parent.session_id,
                error = %e,
                "resolving parent call from child completion failed"
            );
            false
        }
    }
}

/// How a parked sub-agent call should settle based on its child's turn
/// record (`None` = the child is live; leave the call to the pending
/// timeout). A dead child — record gone, session moved on to another turn, or
/// turn already terminal with its resolve lost — settles the call immediately
/// with `(status, result, reason)` for [`resolve_parent`], re-delivering a
/// finalized child's own outcome instead of a generic timeout (MOT-3856).
fn child_settlement(
    child: Option<&crate::types::turn::TurnRecord>,
    expected_turn_id: &str,
) -> Option<(&'static str, Option<Value>, Option<String>)> {
    let Some(rec) = child else {
        return Some((
            "child_lost",
            None,
            Some("child turn record missing — the child died without resolving".to_string()),
        ));
    };
    if rec.turn_id != expected_turn_id {
        return Some((
            "child_lost",
            None,
            Some("child session moved on to another turn without resolving".to_string()),
        ));
    }
    match rec.status {
        TurnStatus::Completed => Some(("completed", rec.result.clone(), None)),
        TurnStatus::Failed => Some((
            "failed",
            None,
            Some(
                rec.result_error
                    .clone()
                    .unwrap_or_else(|| "child turn failed".to_string()),
            ),
        )),
        TurnStatus::Cancelled => Some(("cancelled", None, Some("child turn cancelled".to_string()))),
        TurnStatus::Running | TurnStatus::AwaitingFunctions => None,
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

/// Settle parked pending calls: a call whose child turn is dead (record gone,
/// session moved on, or already terminal with its resolve lost) settles
/// immediately with the child's outcome; a call past its `pending_timeout_ms`
/// resolves with a timeout error. Hook holds (`held_by`) never expire.
pub async fn sweep_expired(deps: &Deps) -> Result<u64, HarnessError> {
    let cfg = deps.cfg().await;
    let records = crate::state::list_turns(&deps.iii, cfg.session_timeout_ms).await?;
    let now = AgentMessage::now_ms();

    // Child lookup from the same snapshot: turn records are keyed one per
    // session, and a child is seeded (put_turn) before its parent checkpoints
    // the call as pending, so a child absent here is genuinely gone.
    let by_session: std::collections::HashMap<&str, &crate::types::turn::TurnRecord> =
        records.iter().map(|r| (r.session_id.as_str(), r)).collect();

    // Collect settlements first; resolve re-reads under each session lock.
    type Settlement = (
        crate::types::turn::ParentLink,
        &'static str,
        Option<Value>,
        Option<String>,
    );
    let mut settle: Vec<Settlement> = Vec::new();
    for record in &records {
        if record.status != TurnStatus::AwaitingFunctions {
            continue;
        }
        for (call_id, cp) in &record.calls {
            if cp.state != CallState::Pending || cp.held_by.is_some() {
                continue;
            }
            let link = crate::types::turn::ParentLink {
                session_id: record.session_id.clone(),
                turn_id: record.turn_id.clone(),
                function_call_id: call_id.clone(),
            };
            if let (Some(cs), Some(ct)) = (&cp.child_session_id, &cp.child_turn_id) {
                if let Some((status, result, reason)) =
                    child_settlement(by_session.get(cs.as_str()).copied(), ct)
                {
                    settle.push((link, status, result, reason));
                    continue;
                }
            }
            if pending_call_expired(cp, cfg.default_pending_timeout_ms, now) {
                settle.push((
                    link,
                    "pending_timeout",
                    None,
                    Some("pending call timed out".to_string()),
                ));
            }
        }
    }

    let mut resolved = 0;
    for (link, status, result, reason) in settle {
        if resolve_parent(deps, &link, status, result.as_ref(), reason.as_deref()).await {
            resolved += 1;
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

fn union_roots(session_roots: Vec<String>, fs_scope_grants: Vec<String>) -> Vec<String> {
    session_roots
        .into_iter()
        .chain(fs_scope_grants)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
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

    fn child_record(turn_id: &str, status: TurnStatus) -> crate::types::turn::TurnRecord {
        crate::types::turn::TurnRecord {
            turn_id: turn_id.into(),
            session_id: "s_child".into(),
            status,
            step: 1,
            turn_count: 1,
            depth: 1,
            abort: false,
            watermark_entry_id: None,
            stream_request_id: None,
            options: crate::types::turn::TurnOptions {
                model: "m".into(),
                provider: None,
                system_prompt: None,
                mode: None,
                max_turns: 16,
                thinking_level: None,
                output: Default::default(),
                functions: None,
                metadata: None,
                max_validation_retries: 2,
            },
            calls: Default::default(),
            parent: None,
            display_parent_session_id: None,
            spawned_by_subscription_id: None,
            reactive_depth: None,
            result: None,
            result_error: None,
            validation_retries: 0,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn missing_child_record_settles_the_parent_call_as_lost() {
        // The wedge class from MOT-3856: the child died without resolving
        // (state lost across a restart, record deleted) — the parent must not
        // wait out the full pending timeout.
        let (status, result, reason) = child_settlement(None, "t_child").expect("settles");
        assert_eq!(status, "child_lost");
        assert!(result.is_none());
        assert!(reason.unwrap().contains("missing"));
    }

    #[test]
    fn child_session_on_a_different_turn_settles_as_lost() {
        let rec = child_record("t_other", TurnStatus::Running);
        let (status, ..) = child_settlement(Some(&rec), "t_child").expect("settles");
        assert_eq!(status, "child_lost");
    }

    #[test]
    fn terminal_child_settles_with_its_own_outcome() {
        // A finalized child whose resolve was lost: re-deliver its outcome
        // rather than a generic timeout.
        let mut done = child_record("t_child", TurnStatus::Completed);
        done.result = Some(serde_json::json!("the answer"));
        let (status, result, reason) = child_settlement(Some(&done), "t_child").expect("settles");
        assert_eq!(status, "completed");
        assert_eq!(result, Some(serde_json::json!("the answer")));
        assert!(reason.is_none());

        let mut failed = child_record("t_child", TurnStatus::Failed);
        failed.result_error = Some("provider exploded".into());
        let (status, result, reason) = child_settlement(Some(&failed), "t_child").expect("settles");
        assert_eq!(status, "failed");
        assert!(result.is_none());
        assert_eq!(reason.as_deref(), Some("provider exploded"));

        let cancelled = child_record("t_child", TurnStatus::Cancelled);
        let (status, ..) = child_settlement(Some(&cancelled), "t_child").expect("settles");
        assert_eq!(status, "cancelled");
    }

    #[test]
    fn live_child_is_left_to_the_pending_timeout() {
        let running = child_record("t_child", TurnStatus::Running);
        assert!(child_settlement(Some(&running), "t_child").is_none());
        let parked = child_record("t_child", TurnStatus::AwaitingFunctions);
        assert!(child_settlement(Some(&parked), "t_child").is_none());
    }

    #[test]
    fn union_roots_dedupes_session_and_one_shot_roots() {
        assert_eq!(
            union_roots(
                vec!["/session".to_string(), "/shared".to_string()],
                vec!["/once".to_string(), "/shared".to_string()],
            ),
            vec![
                "/once".to_string(),
                "/session".to_string(),
                "/shared".to_string()
            ]
        );
    }
}
