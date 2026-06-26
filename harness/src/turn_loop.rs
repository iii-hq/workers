//! The durable turn loop (harness.md § The loop). One `harness::turn` step
//! assembles context, generates one assistant message, dispatches any function
//! calls, then either re-enqueues (model reacts to results / steering) or
//! finalises the turn. Steps are at-least-once: the stale-step guard,
//! deterministic entry ids, and per-call checkpoints make redelivery safe.

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, TriggerAction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::clients::router::{ChatParams, StreamSink};
use crate::clients::{LoadedEntry, SessionClient};
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::ids;
use crate::policy::{self, CallKind, CompiledPolicy};
use crate::trigger;
use crate::types::content::ContentBlock;
use crate::types::message::{empty_assistant, AgentMessage, AssistantMessage};
use crate::types::turn::{CallCheckpoint, CallState, ExposeMode, TurnRecord, TurnStatus};

pub const TURN_QUEUE: &str = "default";

/// The enqueued `harness::turn` step payload.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TurnStepPayload {
    pub session_id: String,
    pub turn_id: String,
    pub step: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TurnStepResult {
    pub session_id: String,
    pub status: TurnStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_step: Option<u64>,
    /// True when a redelivered/stale step was acked and dropped.
    #[serde(default)]
    pub skipped: bool,
}

/// Enqueue the next durable loop step onto the engine's `default` queue.
pub async fn enqueue_step(
    iii: &IIIClient,
    session_id: &str,
    turn_id: &str,
    step: u64,
) -> Result<(), HarnessError> {
    iii.trigger(TriggerRequest {
        function_id: "harness::turn".to_string(),
        payload: json!({ "session_id": session_id, "turn_id": turn_id, "step": step }),
        action: Some(TriggerAction::Enqueue {
            queue: TURN_QUEUE.to_string(),
        }),
        timeout_ms: None,
    })
    .await
    .map(|_| ())
    .map_err(|e| HarnessError::Dependency(format!("enqueue harness::turn: {e}")))
}

fn origin(turn_id: &str) -> Value {
    json!({ "turn_id": turn_id })
}

/// `{ turn_id }` with hook annotations merged in (audit trail — harness.md §
/// Cautions: mutations are silent; annotations record what ran).
fn origin_with(turn_id: &str, annotations: &serde_json::Map<String, Value>) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("turn_id".to_string(), json!(turn_id));
    for (k, v) in annotations {
        obj.insert(k.clone(), v.clone());
    }
    Value::Object(obj)
}

/// Whether the running step must finalise as cancelled after generation.
///
/// Three independent signals mean "cancel": the loop's own in-memory abort flag
/// (a stop observed between steps), a `durable_abort` that a concurrent
/// `harness::stop` wrote to state during generation while the in-memory copy
/// stayed stale, or a stream that itself ended `Aborted`. The middle term is the
/// fix for the post-generation race: a stop landing after a normal `Done` frame
/// carries neither a stale-true local flag nor an `Aborted` stop_reason, so
/// without re-reading durable state it would leak a step of tool execution.
fn cancel_requested(
    local_abort: bool,
    durable_abort: bool,
    stop_reason: crate::types::event::StopReason,
) -> bool {
    local_abort || durable_abort || stop_reason == crate::types::event::StopReason::Aborted
}

/// Run one durable loop step.
pub async fn run_step(
    deps: &Deps,
    payload: TurnStepPayload,
) -> Result<TurnStepResult, HarnessError> {
    // Serialize against off-queue resolves/sweeps on the same session.
    let _guard = deps.locks.guard(&payload.session_id).await;
    let cfg = deps.cfg().await;
    let session = deps.session().await;

    let mut record =
        match crate::state::get_turn(&deps.iii, &payload.session_id, cfg.session_timeout_ms).await?
        {
            Some(r) => r,
            None => return Ok(skipped(&payload.session_id)),
        };

    // Stale guards: wrong turn or an old step is acked and dropped.
    if record.turn_id != payload.turn_id || payload.step < record.step {
        return Ok(skipped(&payload.session_id));
    }
    if record.status.is_terminal() {
        return Ok(skipped(&payload.session_id));
    }

    // Cooperative cancellation observed between steps.
    if record.abort {
        return finalize_cancelled(deps, &session, &mut record, "cancelled").await;
    }

    // First-step bookkeeping: mark working + emit turn-started + pre_turn hook.
    let _ = session
        .set_status(&record.session_id, "working", None)
        .await;
    if payload.step == 0 && record.turn_count == 0 {
        deps.events
            .emit_started(&record.session_id, &record.turn_id, record.parent.as_ref())
            .await;
        if let Err(reason) = deps.hooks.run_pre_turn(&record, payload.step).await {
            return finalize_failed(
                deps,
                &session,
                &mut record,
                &format!("pre_turn hook denied: {reason}"),
            )
            .await;
        }
    }

    // max_turns guard: cap runaway loops with a synthetic notice.
    if record.turn_count >= record.options.max_turns {
        let notice = format!(
            "max_turns ({}) reached; ending the turn.",
            record.options.max_turns
        );
        let _ = session
            .append_custom(
                &record.session_id,
                "notice",
                json!({ "reason": "max_turns", "message": notice }),
                &format!("e_{}_max_turns", record.turn_id),
                Some(&origin(&record.turn_id)),
            )
            .await;
        let text = json!(notice);
        return finalize_completed(deps, &session, &mut record, Some(text), None).await;
    }

    // Load the active path (custom entries carry the compaction record).
    let entries = session.messages(&record.session_id, true).await?;
    let watermark = entries.last().map(|e| e.entry_id.clone());
    record.watermark_entry_id = watermark.clone();

    // Assemble the model-ready context (+ compaction persistence).
    let assembled = assemble_context(deps, &session, &record, &entries, payload.step).await?;

    // Resolve the output-contract strategy and build the invocation surface:
    // the exposure-mode tools plus the synthetic submit_result schema when the
    // contract uses the fallback.
    let strategy = crate::contract::OutputStrategy::resolve(deps, &record).await;
    let mut tools = build_tools(deps, &record).await;
    if let Some(submit) = strategy.submit_result_tool() {
        tools.push(submit);
    }

    // pre_generate hooks: extend the system prompt / append bounded messages,
    // or veto. Annotations ride the assistant entry's origin (audit trail).
    let (gen_system_prompt, appended, gen_annotations) = match deps
        .hooks
        .run_pre_generate(
            &record,
            payload.step,
            assembled.system_prompt.clone(),
            &assembled.messages,
        )
        .await
    {
        crate::hooks::runner::PreGenerateOutcome::Continue {
            system_prompt,
            append_messages,
            annotations,
        } => (system_prompt, append_messages, annotations),
        crate::hooks::runner::PreGenerateOutcome::Deny(reason) => {
            return finalize_failed(
                deps,
                &session,
                &mut record,
                &format!("pre_generate hook denied: {reason}"),
            )
            .await;
        }
    };
    let mut gen_messages = assembled.messages.clone();
    gen_messages.extend(appended);

    // Never hand the provider an empty messages array (Anthropic 400:
    // "messages: at least one message is required"). Assembly's own guards
    // make this unreachable in practice; if it still happens (e.g. a
    // transcript with no user message at all), fail the turn with a clear
    // harness error instead of emitting a cryptic provider error.
    if gen_messages.is_empty() {
        return finalize_failed(
            deps,
            &session,
            &mut record,
            "assembled context is empty; refusing to call the provider with no messages",
        )
        .await;
    }

    let assistant_origin = origin_with(&record.turn_id, &gen_annotations);

    // Generate: append an empty assistant under a deterministic id, stream
    // deltas into it, then write the final message.
    let assistant_id = ids::assistant_entry_id(&record.turn_id, payload.step);
    let provider = record.options.provider.clone().unwrap_or_default();
    let empty = empty_assistant(&provider, &record.options.model);
    let _ = session
        .append(
            &record.session_id,
            &AgentMessage::Assistant(empty),
            Some(&assistant_id),
            None,
            Some(&assistant_origin),
        )
        .await?;

    let sink = SessionStreamSink {
        session: session.clone(),
        session_id: record.session_id.clone(),
        entry_id: assistant_id.clone(),
        turn_id: record.turn_id.clone(),
    };
    let params = ChatParams {
        request_id: format!("{}:{}", record.turn_id, payload.step),
        model: record.options.model.clone(),
        provider: record.options.provider.clone(),
        system_prompt: gen_system_prompt,
        messages: gen_messages,
        tools,
        response_format: strategy.response_format(),
        thinking_level: record.options.thinking_level,
    };
    record.stream_request_id = Some(params.request_id.clone());
    crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;

    // Release the per-session lock across the generation RPC. The loop writes no
    // turn record between here and the post-generation cancellation check
    // (router::chat streams into the SESSION, not the turn record), so dropping
    // the lock lets a concurrent harness::stop take it and set the durable abort
    // flag — observed THIS step by the re-read below — instead of leaving the
    // stop blocked behind the whole step and a step of tool execution leaking.
    drop(_guard);

    let router = deps.router().await;
    let outcome = router.chat(params, &sink).await?;

    // Persist the final assistant message into the streamed entry.
    let _ = session
        .update_message(
            &record.session_id,
            &assistant_id,
            &outcome.message.content,
            None,
            Some(&assistant_origin),
        )
        .await?;

    // post_generate hooks: observe only (the message already streamed).
    deps.hooks
        .run_post_generate(
            &record,
            payload.step,
            serde_json::to_value(&outcome.message).unwrap_or(Value::Null),
        )
        .await;

    // Re-acquire the lock before any further turn-record write, then re-read the
    // durable abort flag under it (authoritative — no concurrent writer once
    // held). A harness::stop that landed during generation set the flag on
    // durable state while this in-memory `record` stayed stale; if the stream
    // had already completed normally it carries stop_reason != Aborted, so
    // without this re-read the stop would be missed until the next step.
    let _guard = deps.locks.guard(&payload.session_id).await;
    let durable_abort =
        crate::state::get_turn(&deps.iii, &payload.session_id, cfg.session_timeout_ms)
            .await?
            .map(|r| r.abort)
            .unwrap_or(false);

    record.turn_count += 1;

    // Cancellation during generation finalises the partial as cancelled.
    if cancel_requested(record.abort, durable_abort, outcome.message.stop_reason) {
        record.abort = true;
        return finalize_cancelled(deps, &session, &mut record, "cancelled").await;
    }
    if !outcome.ok {
        let reason = outcome
            .error
            .clone()
            .unwrap_or_else(|| "generation failed".to_string());
        return finalize_failed(deps, &session, &mut record, &reason).await;
    }

    // Trigger any function calls in content order.
    let expose = record
        .options
        .functions
        .as_ref()
        .map(|f| f.expose)
        .unwrap_or(ExposeMode::AgentTrigger);
    let planned = policy::plan_calls(&outcome.message, expose);
    let trigger_calls: Vec<_> = planned
        .iter()
        .filter(|c| c.kind == CallKind::Trigger)
        .collect();
    let submit_call = planned.iter().find(|c| c.kind == CallKind::SubmitResult);

    if !trigger_calls.is_empty() {
        let policy = CompiledPolicy::from(record.options.functions.as_ref());
        let engine = deps.engine().await;
        for call in trigger_calls.iter().copied() {
            // Per-call checkpoint: skip done/pending, recover an interrupted
            // trigger.
            match record.calls.get(&call.id).map(|c| c.state) {
                Some(CallState::Done) | Some(CallState::Pending) => continue,
                Some(CallState::Triggered) => {
                    append_interrupted(&session, &record, call).await?;
                    let eid = record_entry_id(&record, &call.id);
                    mark_done(&mut record, &call.id, &eid);
                    crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
                    continue;
                }
                _ => {}
            }

            // Fail-closed glob policy first — structural and final. Hooks run
            // only after it passes (a denial never reaches a hook).
            if !policy.allows(&call.function_id) {
                let data = trigger::denied_result(&call.function_id);
                let entry_id = ids::function_result_entry_id(&record.turn_id, &call.id);
                append_function_result(
                    &session,
                    &record,
                    call,
                    &data,
                    &entry_id,
                    &origin(&record.turn_id),
                )
                .await?;
                mark_done(&mut record, &call.id, &entry_id);
                crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
                continue;
            }

            // pre_trigger chain: deny / hold / rewrite arguments.
            let (mut eff_args, pre_ann) = match deps
                .hooks
                .run_pre_trigger(
                    &record,
                    payload.step,
                    &call.id,
                    &call.function_id,
                    &call.arguments,
                )
                .await
            {
                crate::hooks::runner::PreTriggerOutcome::Continue {
                    arguments,
                    annotations,
                } => (arguments, annotations),
                crate::hooks::runner::PreTriggerOutcome::Deny(reason) => {
                    let data = trigger::ResultData {
                        content: vec![ContentBlock::text(reason.clone())],
                        is_error: true,
                        details: json!({ "error": "hook_denied", "message": reason }),
                    };
                    let entry_id = ids::function_result_entry_id(&record.turn_id, &call.id);
                    append_function_result(
                        &session,
                        &record,
                        call,
                        &data,
                        &entry_id,
                        &origin(&record.turn_id),
                    )
                    .await?;
                    mark_done(&mut record, &call.id, &entry_id);
                    crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
                    continue;
                }
                crate::hooks::runner::PreTriggerOutcome::Hold { held_by, .. } => {
                    let info = trigger::PendingInfo {
                        pending_timeout_ms: None,
                        held_by: Some(held_by),
                        child_session_id: None,
                        child_turn_id: None,
                    };
                    checkpoint_pending(&mut record, &call.id, call, &info);
                    crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
                    continue;
                }
            };

            // harness::spawn is the built-in pending trigger: seed a child and
            // park (never invoke a target). Guard failures skip post_trigger.
            if call.function_id == crate::functions::SPAWN_ID {
                match crate::subagent::spawn_pending(deps, &record, &call.id, &eff_args).await {
                    Ok(info) => {
                        checkpoint_pending(&mut record, &call.id, call, &info);
                        crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
                    }
                    Err(data) => {
                        let entry_id = ids::function_result_entry_id(&record.turn_id, &call.id);
                        append_function_result(
                            &session,
                            &record,
                            call,
                            &data,
                            &entry_id,
                            &origin(&record.turn_id),
                        )
                        .await?;
                        mark_done(&mut record, &call.id, &entry_id);
                        crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
                    }
                }
                continue;
            }

            // Checkpoint triggered before invoking the (at-most-once) target.
            record.calls.insert(
                call.id.clone(),
                CallCheckpoint {
                    state: CallState::Triggered,
                    function_id: Some(call.function_id.clone()),
                    entry_id: None,
                    child_session_id: None,
                    child_turn_id: None,
                    held_by: None,
                    pending_timeout_ms: None,
                    pending_at: None,
                },
            );
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;

            // Stamp the owning session onto a subscription control call so the
            // model can never widen the target (no-op for other functions).
            crate::subscriptions::inject_owner_session(
                &call.function_id,
                &mut eff_args,
                &record.session_id,
            );

            // Stamp the per-session working directory onto scoped shell/coder
            // calls as `base_dir` BEFORE invocation. The harness owns scoping:
            // this overwrites any model-supplied base_dir so the model cannot
            // widen its own workspace. A None working_dir is a no-op (the args
            // pass through byte-for-byte unchanged).
            let working_dir = record
                .options
                .metadata
                .as_ref()
                .and_then(|m| m.get("working_dir"))
                .and_then(Value::as_str);
            let scoped_args =
                crate::workspace_inject::inject(&call.function_id, eff_args, working_dir);

            // Invoke the target, then run the post_trigger chain over the
            // result before it is appended (redaction / truncation).
            let raw =
                trigger::invoke_target(&engine, &policy, &call.function_id, &scoped_args).await;
            let (data, post_ann) = deps
                .hooks
                .run_post_trigger(&record, payload.step, &call.id, &call.function_id, raw)
                .await;
            let mut annotations = pre_ann;
            for (k, v) in post_ann {
                annotations.insert(k, v);
            }
            let entry_origin = origin_with(&record.turn_id, &annotations);
            let entry_id = ids::function_result_entry_id(&record.turn_id, &call.id);
            append_function_result(&session, &record, call, &data, &entry_id, &entry_origin)
                .await?;
            mark_done(&mut record, &call.id, &entry_id);
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
        }
    }

    // If any call deferred, the turn parks: no re-enqueue while the deferred
    // work runs (harness.md § Deferred trigger).
    let has_pending = record.calls.values().any(|c| c.state == CallState::Pending);
    if has_pending {
        record.status = TurnStatus::AwaitingFunctions;
        record.updated_at = AgentMessage::now_ms();
        crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
        return Ok(TurnStepResult {
            session_id: record.session_id.clone(),
            status: TurnStatus::AwaitingFunctions,
            next_step: None,
            skipped: false,
        });
    }

    // submit_result is terminal: triggered calls already landed above; now
    // the harness consumes the submit_result (validate + record + finalise).
    if let Some(submit) = submit_call {
        return handle_submit(deps, &session, &mut record, &strategy, submit).await;
    }

    // With triggered calls and no submit_result, re-enqueue so the model
    // reacts to the results.
    if !trigger_calls.is_empty() {
        return advance(deps, &mut record).await;
    }

    // No function calls: steering check, then finalise per the contract.
    if has_user_after_watermark(&session, &record).await? {
        return advance(deps, &mut record).await;
    }

    finalize_with_contract(deps, &session, &mut record, &strategy, &outcome.message).await
}

/// Consume a `submit_result` call: validate its arguments against the
/// contract, record the result, and finalise — or nudge a retry.
async fn handle_submit(
    deps: &Deps,
    session: &SessionClient,
    record: &mut TurnRecord,
    strategy: &crate::contract::OutputStrategy,
    submit: &policy::PlannedCall,
) -> Result<TurnStepResult, HarnessError> {
    let value = submit.arguments.clone();
    match crate::contract::validate_json(&value, strategy.schema()) {
        Ok(()) => finalize_completed(deps, session, record, Some(value), None).await,
        Err(msg) => retry_or_giveup(deps, session, record, &msg, value).await,
    }
}

/// Finalise per the output contract when the model stopped without a function
/// call (text result, provider-native JSON, or a missing submit_result).
async fn finalize_with_contract(
    deps: &Deps,
    session: &SessionClient,
    record: &mut TurnRecord,
    strategy: &crate::contract::OutputStrategy,
    message: &AssistantMessage,
) -> Result<TurnStepResult, HarnessError> {
    let text = ContentBlock::join_text(&message.content);
    match strategy {
        crate::contract::OutputStrategy::Text => {
            finalize_completed(deps, session, record, Some(json!(text)), None).await
        }
        crate::contract::OutputStrategy::ProviderNativeJson { schema } => {
            match crate::contract::parse_json_text(&text) {
                Ok(value) => match crate::contract::validate_json(&value, schema.as_ref()) {
                    Ok(()) => finalize_completed(deps, session, record, Some(value), None).await,
                    Err(msg) => retry_or_giveup(deps, session, record, &msg, json!(text)).await,
                },
                Err(msg) => retry_or_giveup(deps, session, record, &msg, json!(text)).await,
            }
        }
        crate::contract::OutputStrategy::SubmitResultJson { .. } => {
            let msg = "this turn must produce its result by calling submit_result; \
                       no submit_result call was made"
                .to_string();
            retry_or_giveup(deps, session, record, &msg, json!(text)).await
        }
    }
}

/// Nudge a validation retry (bounded by `max_validation_retries`); after the
/// budget, finalise `completed` with `result_error` and the best-effort
/// result.
async fn retry_or_giveup(
    deps: &Deps,
    session: &SessionClient,
    record: &mut TurnRecord,
    error: &str,
    fallback_result: Value,
) -> Result<TurnStepResult, HarnessError> {
    if record.validation_retries < record.options.max_validation_retries {
        let attempt = record.validation_retries + 1;
        let nudge = AgentMessage::user_text(format!(
            "Your previous result was not accepted: {error}. Please provide a corrected result \
             that satisfies the required output contract."
        ));
        let _ = session
            .append(
                &record.session_id,
                &nudge,
                Some(&ids::validation_nudge_entry_id(&record.turn_id, attempt)),
                None,
                Some(&origin(&record.turn_id)),
            )
            .await?;
        record.validation_retries = attempt;
        advance(deps, record).await
    } else {
        finalize_completed(
            deps,
            session,
            record,
            Some(fallback_result),
            Some(error.to_string()),
        )
        .await
    }
}

/// Persist the advanced step and re-enqueue the next `harness::turn`.
async fn advance(deps: &Deps, record: &mut TurnRecord) -> Result<TurnStepResult, HarnessError> {
    let cfg = deps.cfg().await;
    let next = record.step + 1;
    record.step = next;
    record.status = TurnStatus::Running;
    record.updated_at = AgentMessage::now_ms();
    crate::state::put_turn(&deps.iii, record, cfg.session_timeout_ms).await?;
    enqueue_step(&deps.iii, &record.session_id, &record.turn_id, next).await?;
    Ok(TurnStepResult {
        session_id: record.session_id.clone(),
        status: TurnStatus::Running,
        next_step: Some(next),
        skipped: false,
    })
}

async fn finalize_completed(
    deps: &Deps,
    session: &SessionClient,
    record: &mut TurnRecord,
    result: Option<Value>,
    result_error: Option<String>,
) -> Result<TurnStepResult, HarnessError> {
    let cfg = deps.cfg().await;
    record.status = TurnStatus::Completed;
    record.result = result.clone();
    record.result_error = result_error.clone();
    record.updated_at = AgentMessage::now_ms();
    crate::state::put_turn(&deps.iii, record, cfg.session_timeout_ms).await?;
    let _ = session.set_status(&record.session_id, "done", None).await;
    deps.events
        .emit_completed(
            &record.session_id,
            &record.turn_id,
            "completed",
            result.as_ref(),
            result_error.as_deref(),
            None,
            record.parent.as_ref(),
        )
        .await;
    // Sub-agent turns resolve the parent's pending call with their result.
    if let Some(parent) = record.parent.clone() {
        crate::deferred::resolve_parent(deps, &parent, "completed", result.as_ref(), None).await;
    }
    Ok(TurnStepResult {
        session_id: record.session_id.clone(),
        status: TurnStatus::Completed,
        next_step: None,
        skipped: false,
    })
}

async fn finalize_failed(
    deps: &Deps,
    session: &SessionClient,
    record: &mut TurnRecord,
    reason: &str,
) -> Result<TurnStepResult, HarnessError> {
    let cfg = deps.cfg().await;
    record.status = TurnStatus::Failed;
    record.result_error = Some(reason.to_string());
    record.updated_at = AgentMessage::now_ms();
    crate::state::put_turn(&deps.iii, record, cfg.session_timeout_ms).await?;
    let _ = session
        .append_custom(
            &record.session_id,
            "error",
            json!({ "reason": reason }),
            &format!("e_{}_error", record.turn_id),
            Some(&origin(&record.turn_id)),
        )
        .await;
    let _ = session
        .set_status(&record.session_id, "error", Some(reason))
        .await;
    deps.events
        .emit_completed(
            &record.session_id,
            &record.turn_id,
            "failed",
            None,
            None,
            Some(reason),
            record.parent.as_ref(),
        )
        .await;
    if let Some(parent) = record.parent.clone() {
        crate::deferred::resolve_parent(deps, &parent, "failed", None, Some(reason)).await;
    }
    Ok(TurnStepResult {
        session_id: record.session_id.clone(),
        status: TurnStatus::Failed,
        next_step: None,
        skipped: false,
    })
}

async fn finalize_cancelled(
    deps: &Deps,
    session: &SessionClient,
    record: &mut TurnRecord,
    reason: &str,
) -> Result<TurnStepResult, HarnessError> {
    let cfg = deps.cfg().await;
    record.status = TurnStatus::Cancelled;
    record.updated_at = AgentMessage::now_ms();
    crate::state::put_turn(&deps.iii, record, cfg.session_timeout_ms).await?;
    let _ = session.set_status(&record.session_id, "done", None).await;
    deps.events
        .emit_completed(
            &record.session_id,
            &record.turn_id,
            "cancelled",
            None,
            None,
            Some(reason),
            record.parent.as_ref(),
        )
        .await;
    if let Some(parent) = record.parent.clone() {
        crate::deferred::resolve_parent(deps, &parent, "cancelled", None, Some(reason)).await;
    }
    Ok(TurnStepResult {
        session_id: record.session_id.clone(),
        status: TurnStatus::Cancelled,
        next_step: None,
        skipped: false,
    })
}

/// Finalise a turn as failed after an unexpected step error (harness.md §
/// `harness::turn` failure handling). Idempotent: a terminal turn is left
/// untouched.
pub async fn fail_turn(
    deps: &Deps,
    session_id: &str,
    turn_id: &str,
    reason: &str,
) -> TurnStepResult {
    let cfg = deps.cfg().await;
    let session = deps.session().await;
    let record = crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms)
        .await
        .ok()
        .flatten();
    match record {
        Some(mut rec) if rec.turn_id == turn_id && !rec.status.is_terminal() => {
            finalize_failed(deps, &session, &mut rec, reason)
                .await
                .unwrap_or_else(|_| skipped(session_id))
        }
        _ => skipped(session_id),
    }
}

fn skipped(session_id: &str) -> TurnStepResult {
    TurnStepResult {
        session_id: session_id.to_string(),
        status: TurnStatus::Running,
        next_step: None,
        skipped: true,
    }
}

fn checkpoint_pending(
    record: &mut TurnRecord,
    call_id: &str,
    call: &policy::PlannedCall,
    info: &trigger::PendingInfo,
) {
    record.calls.insert(
        call_id.to_string(),
        CallCheckpoint {
            state: CallState::Pending,
            function_id: Some(call.function_id.clone()),
            entry_id: None,
            child_session_id: info.child_session_id.clone(),
            child_turn_id: info.child_turn_id.clone(),
            held_by: info.held_by.clone(),
            pending_timeout_ms: info.pending_timeout_ms,
            pending_at: Some(AgentMessage::now_ms()),
        },
    );
}

fn mark_done(record: &mut TurnRecord, call_id: &str, entry_id: &str) {
    record.calls.insert(
        call_id.to_string(),
        CallCheckpoint {
            state: CallState::Done,
            function_id: record
                .calls
                .get(call_id)
                .and_then(|c| c.function_id.clone()),
            entry_id: Some(entry_id.to_string()),
            child_session_id: None,
            child_turn_id: None,
            held_by: None,
            pending_timeout_ms: None,
            pending_at: None,
        },
    );
}

fn record_entry_id(record: &TurnRecord, call_id: &str) -> String {
    ids::function_result_entry_id(&record.turn_id, call_id)
}

async fn append_function_result(
    session: &SessionClient,
    record: &TurnRecord,
    call: &policy::PlannedCall,
    data: &trigger::ResultData,
    entry_id: &str,
    origin: &Value,
) -> Result<(), HarnessError> {
    let message = AgentMessage::FunctionResult(crate::types::message::FunctionResultMessage {
        role: crate::types::message::FunctionResultRoleTag::FunctionResult,
        function_call_id: call.id.clone(),
        function_id: call.function_id.clone(),
        content: data.content.clone(),
        details: data.details.clone(),
        is_error: data.is_error,
        timestamp: AgentMessage::now_ms(),
    });
    session
        .append(
            &record.session_id,
            &message,
            Some(entry_id),
            None,
            Some(origin),
        )
        .await
        .map(|_| ())
}

async fn append_interrupted(
    session: &SessionClient,
    record: &TurnRecord,
    call: &policy::PlannedCall,
) -> Result<(), HarnessError> {
    let data = trigger::ResultData {
        content: vec![ContentBlock::text(
            "interrupted: executed at most once, result unknown (restart during execution)"
                .to_string(),
        )],
        is_error: true,
        details: json!({ "error": "interrupted" }),
    };
    let entry_id = ids::function_result_entry_id(&record.turn_id, &call.id);
    append_function_result(
        session,
        record,
        call,
        &data,
        &entry_id,
        &origin(&record.turn_id),
    )
    .await
}

/// Steering: are there user-role entries after the assemble-time watermark?
async fn has_user_after_watermark(
    session: &SessionClient,
    record: &TurnRecord,
) -> Result<bool, HarnessError> {
    let Some(watermark) = &record.watermark_entry_id else {
        return Ok(false);
    };
    let entries = session.messages(&record.session_id, false).await?;
    let mut after = false;
    for entry in entries {
        if after {
            if let Some(AgentMessage::User(_)) = &entry.message {
                return Ok(true);
            }
        }
        if &entry.entry_id == watermark {
            after = true;
        }
    }
    Ok(false)
}

/// Build the model-ready context: read the latest compaction entry, reduce the
/// candidate window to its tail, call `context::assemble` (persisting a new
/// summary when it compacts), or fall back to raw history.
async fn assemble_context(
    deps: &Deps,
    session: &SessionClient,
    record: &TurnRecord,
    entries: &[LoadedEntry],
    step: u64,
) -> Result<Assembled, HarnessError> {
    // Latest compaction custom entry on the path (if any).
    let mut previous_summary: Option<String> = None;
    let mut tail_start: Option<String> = None;
    for entry in entries {
        if let Some(custom) = &entry.custom {
            if custom.custom_type == "compaction" {
                previous_summary = custom
                    .data
                    .get("summary")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                tail_start = custom
                    .data
                    .get("tail_start_entry_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
        }
    }

    // Candidate window: message entries from tail_start onward (compaction
    // entries themselves are never sent to the model).
    let mut started = tail_start.is_none();
    let mut candidate: Vec<(String, AgentMessage)> = Vec::new();
    for entry in entries {
        if let Some(ts) = &tail_start {
            if &entry.entry_id == ts {
                started = true;
            }
        }
        if !started {
            continue;
        }
        if let Some(msg) = &entry.message {
            if !matches!(msg, AgentMessage::Custom(_)) {
                candidate.push((entry.entry_id.clone(), msg.clone()));
            }
        }
    }

    let candidate_values: Vec<Value> = candidate
        .iter()
        .map(|(_, m)| serde_json::to_value(m).unwrap_or(Value::Null))
        .collect();

    let context = deps.context().await;
    let params = crate::clients::AssembleParams {
        messages: candidate_values.clone(),
        model_id: record.options.model.clone(),
        provider: record.options.provider.clone(),
        system_prompt: record.options.system_prompt.clone(),
        previous_summary,
        lease_key: record.session_id.clone(),
        thinking_level: record.options.thinking_level,
    };

    match context.assemble(params).await {
        Ok(Some(out)) => {
            if out.applied.compacted {
                if let Some(summary) = &out.applied.summary {
                    let tail_entry = out
                        .applied
                        .tail_start_index
                        .and_then(|i| usize::try_from(i).ok())
                        .and_then(|i| candidate.get(i))
                        .map(|(id, _)| id.clone());
                    let data = json!({
                        "summary": summary,
                        "tail_start_entry_id": tail_entry,
                        "tokens_before": out.applied.tail_start_index,
                    });
                    let _ = session
                        .append_custom(
                            &record.session_id,
                            "compaction",
                            data,
                            &ids::compaction_entry_id(&record.turn_id, step),
                            Some(&origin(&record.turn_id)),
                        )
                        .await;
                }
            }
            let mut messages: Vec<Value> = out
                .messages
                .iter()
                .map(|m| serde_json::to_value(m).unwrap_or(Value::Null))
                .collect();
            // Defensive: context::assemble must never strip the model-facing
            // context down to nothing (the provider rejects an empty messages
            // array). If it somehow does, fall back to the raw candidate
            // window — which still begins at a user message.
            if messages.is_empty() && !candidate_values.is_empty() {
                tracing::warn!(
                    session_id = %record.session_id,
                    "context::assemble returned empty messages; using raw candidate window"
                );
                messages = candidate_values.clone();
            }
            Ok(Assembled {
                system_prompt: with_working_dir_aid(Some(out.system_prompt), record),
                messages,
            })
        }
        Ok(None) | Err(_) => Ok(Assembled {
            system_prompt: with_working_dir_aid(record.options.system_prompt.clone(), record),
            messages: candidate_values,
        }),
    }
}

/// Append a working-directory line to the system prompt when the turn carries a
/// `working_dir` in its metadata. This is a model-facing AID only — the real
/// scoping control plane stamps `base_dir` onto each call
/// (`workspace_inject::inject`); this line just tells the model where it is so
/// it reasons about relative paths sensibly.
fn with_working_dir_aid(system_prompt: Option<String>, record: &TurnRecord) -> Option<String> {
    let working_dir = record
        .options
        .metadata
        .as_ref()
        .and_then(|m| m.get("working_dir"))
        .and_then(Value::as_str);
    let Some(dir) = working_dir else {
        return system_prompt;
    };
    let line = format!("Your working directory is {dir}.");
    Some(match system_prompt {
        Some(prompt) if !prompt.is_empty() => format!("{prompt}\n{line}"),
        _ => line,
    })
}

struct Assembled {
    system_prompt: Option<String>,
    messages: Vec<Value>,
}

/// Build the invocation-schema surface attached to the generate request
/// (harness.md § Exposure modes). Default: the single `agent_trigger` schema.
/// Native: expand the allow globs against the registry and attach one schema
/// per allowed function.
async fn build_tools(deps: &Deps, record: &TurnRecord) -> Vec<crate::types::model::AgentFunction> {
    let expose = record
        .options
        .functions
        .as_ref()
        .map(|f| f.expose)
        .unwrap_or(ExposeMode::AgentTrigger);
    match expose {
        ExposeMode::AgentTrigger => vec![policy::agent_trigger_schema()],
        ExposeMode::Native => {
            let policy = CompiledPolicy::from(record.options.functions.as_ref());
            let snapshot = deps.functions().await;
            let mut tools = Vec::new();
            for descriptor in snapshot.iter() {
                if !policy.allows(&descriptor.function_id) {
                    continue;
                }
                tools.push(crate::types::model::AgentFunction {
                    name: descriptor.function_id.clone(),
                    description: descriptor.description.clone().unwrap_or_default(),
                    parameters: descriptor
                        .parameters
                        .clone()
                        .unwrap_or_else(|| json!({ "type": "object" })),
                    label: None,
                    execution_mode: Some("sequential".to_string()),
                });
            }
            if tools.is_empty() {
                tracing::warn!(
                    session_id = %record.session_id,
                    "native exposure matched no registry functions; the model has no tools this turn"
                );
            }
            tools
        }
    }
}

/// Streams coalesced partials into the assistant entry via
/// `session::update-message`. Failures are logged, never fatal.
struct SessionStreamSink {
    session: SessionClient,
    session_id: String,
    entry_id: String,
    turn_id: String,
}

#[async_trait]
impl StreamSink for SessionStreamSink {
    async fn on_update(&self, message: &AssistantMessage) {
        let origin = origin(&self.turn_id);
        if let Err(e) = self
            .session
            .update_message(
                &self.session_id,
                &self.entry_id,
                &message.content,
                None,
                Some(&origin),
            )
            .await
        {
            tracing::warn!(session_id = %self.session_id, error = %e, "stream update-message failed");
        }
    }
}

impl Clone for SessionStreamSink {
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            session_id: self.session_id.clone(),
            entry_id: self.entry_id.clone(),
            turn_id: self.turn_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::cancel_requested;
    use crate::types::event::StopReason;

    #[test]
    fn durable_abort_is_observed_even_when_local_is_stale_and_stream_completed() {
        // The post-generation race: a harness::stop landed after a normal `Done`
        // frame, so the loop's in-memory flag is still false and the stream's
        // stop_reason is End (not Aborted). Re-reading durable state must still
        // cancel — otherwise this step's tool calls execute despite the stop.
        assert!(cancel_requested(false, true, StopReason::End));
        assert!(cancel_requested(false, true, StopReason::FunctionCall));
    }

    #[test]
    fn local_abort_or_aborted_stream_still_cancels() {
        // A stop observed between steps (local flag) or a stream the router tore
        // down (Aborted) cancel regardless of durable state.
        assert!(cancel_requested(true, false, StopReason::End));
        assert!(cancel_requested(false, false, StopReason::Aborted));
    }

    #[test]
    fn no_signal_does_not_cancel() {
        assert!(!cancel_requested(false, false, StopReason::End));
        assert!(!cancel_requested(false, false, StopReason::FunctionCall));
    }
}
