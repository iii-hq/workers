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
use crate::types::turn::{
    CallCheckpoint, CallState, ExposeMode, FunctionPolicy, TurnRecord, TurnStatus,
};

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
            .emit_started(
                &record.session_id,
                &record.turn_id,
                record.parent.as_ref(),
                record.display_parent_session_id.as_deref(),
                crate::events::ReactiveMeta {
                    spawned_by: record.spawned_by_subscription_id.as_deref(),
                    depth: record.reactive_depth,
                },
            )
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
    // The previous step's watermark marks which entries arrived while that
    // step was generating (assemble_context rotates them past the reply they
    // interrupted — see rotate_mid_generation_users). The new watermark is
    // assigned only AFTER the generate is consumed: the pre-generate put_turn
    // must persist the OLD one, or a redelivered step loses the rotation
    // window and re-issues the prefill-rejected trailing-assistant shape on
    // every retry.
    let prev_watermark = record.watermark_entry_id.clone();
    let watermark = entries.last().map(|e| e.entry_id.clone());

    // Assemble the model-ready context (+ compaction persistence).
    let mut assembled = assemble_context(
        deps,
        &session,
        &record,
        &entries,
        payload.step,
        prev_watermark.as_deref(),
    )
    .await?;

    // Registry-change notice: if the function registry changed since this
    // session last acknowledged its generation, tell the model its cached
    // contracts may be stale. First sighting stamps silently. Persisted by the
    // pre-generate put_turn below.
    let snapshot = deps.functions().await;
    let current_generation = snapshot.generation;
    if let Some(notice) = registry_notice(record.functions_generation, current_generation) {
        assembled.system_prompt = Some(match assembled.system_prompt.take() {
            Some(prompt) if !prompt.is_empty() => format!("{prompt}\n\n{notice}"),
            _ => notice,
        });
    }
    record.functions_generation = Some(current_generation);

    // A compaction may have summarized earlier list results out of the
    // context — the dedup stub is only safe while the original result is
    // still readable, so forget served list queries. Persist immediately:
    // the compaction record is already durable (assemble_context wrote it),
    // and a crash before the pre-generate put_turn must not resurrect
    // entries whose results were summarized away (re-assembly of an
    // already-compacted path reports compacted=false).
    if assembled.compacted && !record.list_queries.is_empty() {
        record.list_queries.clear();
        crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
    }

    // Injected function catalog: the configured slice of the registry
    // snapshot, so common ids need no engine::functions::list round trip.
    // agent_trigger exposure only — native/code-mode tools already carry
    // their own surface. Rebuilt per step from the live snapshot, so it is
    // always current.
    let expose = record
        .options
        .functions
        .as_ref()
        .map(|f| f.expose)
        .unwrap_or(ExposeMode::AgentTrigger);
    if expose == ExposeMode::AgentTrigger {
        let globs = crate::catalog::compile_globs(&cfg.catalog_functions);
        let policy = CompiledPolicy::from(record.options.functions.as_ref());
        if let Some(block) = crate::catalog::render(&snapshot.functions, &globs, &policy) {
            assembled.system_prompt = Some(match assembled.system_prompt.take() {
                Some(prompt) if !prompt.is_empty() => format!("{prompt}\n\n{block}"),
                _ => block,
            });
        }
    }

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

    // Post-assembly invariant guard: providers reject a context where an
    // assistant function_call has no function_result. Compaction can cut a
    // pair (result summarized away, call kept) even when the TRANSCRIPT is
    // fully paired — patch the assembled copy only.
    let patched = patch_orphaned_calls(&mut gen_messages);
    if patched > 0 {
        tracing::warn!(
            session_id = %record.session_id,
            turn_id = %record.turn_id,
            patched,
            "assembled context contained orphaned function_calls; injected elided results (compaction cut a call/result pair)"
        );
    }

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

    // Generation consumed: advance the steering watermark (persisted by the
    // advance()/finalize call that ends this step).
    record.watermark_entry_id = watermark;

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
        let session_grants =
            crate::filesystem_grants::roots(&deps.iii, &record.session_id, cfg.session_timeout_ms)
                .await?;
        let filesystem_root = record.options.filesystem_root().map(str::to_string);
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

            // pre_trigger chain: deny / hold / rewrite arguments. Hooks see
            // args ALREADY carrying the filesystem scope stamp so an approver
            // reviews the fs_scope the call will actually run under; the stamp is
            // re-applied after the chain so a hook rewrite can never widen it.
            let trusted_call_args = crate::filesystem_scope::inject(
                &call.function_id,
                call.arguments.clone(),
                filesystem_root.as_deref(),
                &session_grants,
            );
            let (eff_args, pre_ann) = match deps
                .hooks
                .run_pre_trigger(
                    &record,
                    payload.step,
                    &call.id,
                    &call.function_id,
                    &trusted_call_args,
                )
                .await
            {
                crate::hooks::runner::PreTriggerOutcome::Continue {
                    arguments,
                    annotations,
                } => {
                    let arguments = crate::filesystem_scope::inject(
                        &call.function_id,
                        arguments,
                        filesystem_root.as_deref(),
                        &session_grants,
                    );
                    (arguments, annotations)
                }
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

            // Single invocation chokepoint: subscription control calls are
            // intercepted (trusted session injected); everything else invokes the
            // target. Then the post_trigger chain runs over the result.
            // Same-query list dedup first: a byte-identical
            // engine::functions::list repeat at an unchanged registry
            // generation gets an "unchanged — re-read your earlier result"
            // stub instead of another full catalog dump. The generation is
            // re-read HERE, not reused from step start — generation can take
            // minutes and a registry change in between must not be answered
            // with a stale "unchanged" stub.
            let is_list_call = call.function_id == "engine::functions::list";
            let list_generation = if is_list_call {
                deps.functions().await.generation
            } else {
                0
            };
            let dedup_stub = if is_list_call {
                list_dedup(&record.list_queries, &eff_args, list_generation)
            } else {
                None
            };
            let raw = match dedup_stub {
                Some(stub) => stub,
                None => {
                    crate::functions::subscribe::invoke(
                        deps,
                        &engine,
                        &policy,
                        &call.function_id,
                        &eff_args,
                        &record.session_id,
                    )
                    .await
                }
            };
            let post_outcome = deps
                .hooks
                .run_post_trigger(
                    &record,
                    payload.step,
                    &call.id,
                    &call.function_id,
                    &eff_args,
                    raw,
                )
                .await;
            let mut annotations = pre_ann;
            let (data, post_ann) = match post_outcome {
                crate::hooks::runner::PostTriggerOutcome::Result {
                    result,
                    annotations,
                } => (result, annotations),
                crate::hooks::runner::PostTriggerOutcome::Hold {
                    held_by,
                    annotations: _,
                } => {
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
            for (k, v) in post_ann {
                annotations.insert(k, v);
            }
            let entry_origin = origin_with(&record.turn_id, &annotations);
            let entry_id = ids::function_result_entry_id(&record.turn_id, &call.id);
            append_function_result(&session, &record, call, &data, &entry_id, &entry_origin)
                .await?;
            // Record the served list query only once its SUCCESSFUL result
            // durably exists in the session — a held call or a hook-rewritten
            // error must never poison the dedup with a stub pointing at a
            // result the model cannot read.
            if is_list_call && !data.is_error {
                list_dedup_record(&mut record.list_queries, &eff_args, list_generation);
            }
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
            record.display_parent_session_id.as_deref(),
            crate::events::ReactiveMeta {
                spawned_by: record.spawned_by_subscription_id.as_deref(),
                depth: record.reactive_depth,
            },
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
            record.display_parent_session_id.as_deref(),
            crate::events::ReactiveMeta {
                spawned_by: record.spawned_by_subscription_id.as_deref(),
                depth: record.reactive_depth,
            },
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
            record.display_parent_session_id.as_deref(),
            crate::events::ReactiveMeta {
                spawned_by: record.spawned_by_subscription_id.as_deref(),
                depth: record.reactive_depth,
            },
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
    // include_custom must match the watermark's source list (the step entry
    // loads with `true`): a watermark landing on a custom entry would
    // otherwise never be found and the steering check silently dies.
    let entries = session.messages(&record.session_id, true).await?;
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
    prev_watermark: Option<&str>,
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
    // Index (into `candidate`) of the first entry appended after the previous
    // step's watermark — i.e. while that step was generating.
    let mut first_new: Option<usize> = None;
    let mut past_prev_watermark = false;
    for entry in entries {
        if let Some(ts) = &tail_start {
            if &entry.entry_id == ts {
                started = true;
            }
        }
        if started {
            if let Some(msg) = &entry.message {
                if !matches!(msg, AgentMessage::Custom(_)) {
                    if past_prev_watermark && first_new.is_none() {
                        first_new = Some(candidate.len());
                    }
                    candidate.push((entry.entry_id.clone(), msg.clone()));
                }
            }
        }
        if prev_watermark == Some(entry.entry_id.as_str()) {
            past_prev_watermark = true;
        }
    }

    // Rotation happens on the FINAL assembled values (below), never on
    // `candidate`: compaction bookkeeping maps tail_start_index into
    // `candidate` as a log-order cursor, and rotating first would persist a
    // tail_start_entry_id that silently drops the rotated user message from
    // every future window.
    let new_suffix_len = first_new.map(|i| candidate.len() - i).unwrap_or(0);

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
            rotate_mid_generation_users(&mut messages, new_suffix_len);
            Ok(Assembled {
                system_prompt: with_filesystem_root_aid(Some(out.system_prompt), record),
                messages,
                compacted: out.applied.compacted,
            })
        }
        Ok(None) | Err(_) => {
            let mut messages = candidate_values;
            rotate_mid_generation_users(&mut messages, new_suffix_len);
            Ok(Assembled {
                system_prompt: with_filesystem_root_aid(
                    record.options.system_prompt.clone(),
                    record,
                ),
                messages,
                compacted: false,
            })
        }
    }
}

/// Append model-facing context aid lines to the system prompt: the session id
/// (always — it makes the prompt's "<this session>" recipes actionable, e.g.
/// `turn-completed` filters and reactive spawns that deliver into this chat),
/// the working directory (when the turn carries a `filesystem_root`), and the
/// dispatch-policy surface (when it is narrowed — see [`policy_aid`]). These
/// are AIDs only — the real scoping control plane stamps `fs_scope` onto each
/// call (`filesystem_scope::inject`) and the policy stays fail-closed at
/// dispatch.
fn with_filesystem_root_aid(system_prompt: Option<String>, record: &TurnRecord) -> Option<String> {
    let mut lines = vec![format!("Your session id is {}.", record.session_id)];
    if let Some(dir) = record.options.filesystem_root() {
        lines.push(format!("Your working directory is {dir}."));
    }
    if let Some(aid) = policy_aid(record.options.functions.as_ref()) {
        lines.push(aid);
    }
    let aid = lines.join("\n");
    Some(match system_prompt {
        Some(prompt) if !prompt.is_empty() => format!("{prompt}\n{aid}"),
        _ => aid,
    })
}

/// The dispatch-policy aid line for a narrowed turn, `None` when the surface
/// is unrestricted (a `*` allow — the prompt's discovery doctrine is correct
/// there). A narrowed agent is never otherwise shown its allow-list, so it
/// dutifully follows that doctrine into a denied `engine::functions::list` on
/// its very first step; telling it the exact surface makes discovery moot.
fn policy_aid(policy: Option<&FunctionPolicy>) -> Option<String> {
    const MAX_LISTED: usize = 30;
    let denied_all = "Function dispatch is entirely disabled this turn — do not call any function.";
    let Some(p) = policy else {
        return Some(denied_all.to_string());
    };
    if p.allow.iter().any(|g| g == "*") {
        return None;
    }
    if p.allow.is_empty() {
        return Some(denied_all.to_string());
    }
    let mut allow: Vec<&str> = p.allow.iter().map(String::as_str).collect();
    allow.sort_unstable();
    allow.dedup();
    let over = allow.len() > MAX_LISTED;
    let shown = allow[..allow.len().min(MAX_LISTED)].join(", ");
    let ellipsis = if over { ", …" } else { "" };
    let deny = if p.deny.is_empty() {
        String::new()
    } else {
        format!(" Deny-listed on top: {}.", p.deny.join(", "))
    };
    Some(format!(
        "Your dispatch policy allows ONLY these functions: {shown}{ellipsis}.{deny} Anything \
         else — including discovery (engine::functions::list / ::info) unless listed above — is \
         denied. Do not probe: if the task needs a function not allowed here, say so and finish."
    ))
}

struct Assembled {
    system_prompt: Option<String>,
    messages: Vec<Value>,
    /// Whether THIS assembly compacted the context (a new compaction summary
    /// was persisted): earlier tool results may now be summarized away.
    compacted: bool,
}

/// A user entry appended while a step was generating (or assembling — the
/// compaction/hook window) lands BEFORE that step's assistant entry in the
/// durable log. The steering check then re-generates, but the assembled list
/// would END with the assistant message: a prefill request Anthropic rejects
/// ("This model does not support assistant message prefill. The conversation
/// must end with a user message."), wedging the turn on every retry. Present
/// mid-generation arrivals AFTER the reply they interrupted — semantically
/// exact: the model answered without seeing them. Runs on the FINAL assembled
/// values so compaction bookkeeping stays in log order; `new_suffix_len` is
/// how many trailing messages arrived after the previous step's watermark
/// (only user messages inside that suffix rotate). The window is clamped off
/// the opening message so the context always still starts with the user turn.
/// The durable transcript is untouched.
fn rotate_mid_generation_users(messages: &mut Vec<Value>, new_suffix_len: usize) {
    if new_suffix_len == 0 || messages.len() < 2 {
        return;
    }
    let last = messages.len() - 1;
    let tail = &messages[last];
    let trailing_callless_assistant = tail.get("role").and_then(Value::as_str) == Some("assistant")
        && !tail
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .any(|b| b.get("type").and_then(Value::as_str) == Some("function_call"))
            })
            .unwrap_or(false);
    if !trailing_callless_assistant {
        return;
    }
    let window_start = messages.len().saturating_sub(new_suffix_len).max(1);
    let mut moved: Vec<Value> = Vec::new();
    let mut i = window_start;
    while i < messages.len() - 1 {
        if messages[i].get("role").and_then(Value::as_str) == Some("user") {
            moved.push(messages.remove(i));
        } else {
            i += 1;
        }
    }
    messages.extend(moved);
}

/// Patch an ASSEMBLED message list whose assistant `function_call` blocks lack
/// a `function_result` anywhere in the list — the shape providers hard-reject
/// (`tool_use` without `tool_result`). Injects a synthetic "elided" result
/// message directly after each orphaned call's assistant message. Returns how
/// many results were injected. The durable transcript is never touched; the
/// orphan usually means compaction cut a call/result pair.
fn patch_orphaned_calls(messages: &mut Vec<Value>) -> usize {
    let mut resolved: std::collections::HashSet<String> = std::collections::HashSet::new();
    for m in messages.iter() {
        if let Some(id) = m.get("function_call_id").and_then(Value::as_str) {
            resolved.insert(id.to_string());
        }
        if let Some(blocks) = m.get("content").and_then(Value::as_array) {
            for b in blocks {
                if b.get("type").and_then(Value::as_str) == Some("function_result") {
                    if let Some(id) = b.get("function_call_id").and_then(Value::as_str) {
                        resolved.insert(id.to_string());
                    }
                }
            }
        }
    }

    let mut patched = 0usize;
    let mut i = 0;
    while i < messages.len() {
        let mut missing: Vec<(String, String)> = Vec::new();
        if messages[i].get("role").and_then(Value::as_str) == Some("assistant") {
            if let Some(blocks) = messages[i].get("content").and_then(Value::as_array) {
                for b in blocks {
                    if b.get("type").and_then(Value::as_str) != Some("function_call") {
                        continue;
                    }
                    let Some(id) = b.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    if resolved.contains(id) {
                        continue;
                    }
                    let fid = b
                        .get("function_id")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    missing.push((id.to_string(), fid.to_string()));
                }
            }
        }
        let inserted = missing.len();
        for (off, (id, fid)) in missing.into_iter().enumerate() {
            messages.insert(
                i + 1 + off,
                json!({
                    "role": "function_result",
                    "function_call_id": id,
                    "function_id": fid,
                    "content": [{
                        "type": "text",
                        "text": "result elided from the assembled context (compaction); the call completed in an earlier turn — consult the transcript if its output matters",
                    }],
                    "is_error": false,
                    "timestamp": AgentMessage::now_ms(),
                }),
            );
            patched += 1;
        }
        i += 1 + inserted;
    }
    patched
}

/// The single-line notice appended to the system prompt when the registry
/// changed under a session that had already acknowledged an earlier generation.
const REGISTRY_CHANGED_NOTICE: &str = "NOTE: the function registry changed during this conversation. Function contracts and list results you fetched earlier may be stale — re-fetch the contracts you rely on (engine::functions::info) before calling those functions again, and re-list before reusing an earlier list result. (A function catalog block in this prompt is rebuilt fresh each turn and is already current.)";

/// Decide the registry-change notice for a step. `None` when the record already
/// matches the live generation, or is being stamped for the first time; `Some`
/// only when the registry changed under a session that acknowledged an earlier
/// generation. The caller stamps `functions_generation = current` regardless.
fn registry_notice(record_gen: Option<u64>, current: u64) -> Option<String> {
    match record_gen {
        Some(g) if g != current => Some(REGISTRY_CHANGED_NOTICE.to_string()),
        _ => None,
    }
}

/// Cap on remembered `engine::functions::list` payloads per session.
const LIST_QUERIES_MAX: usize = 16;

/// Canonical form of a list payload — serde_json's map is BTreeMap-backed,
/// so serialization sorts keys.
fn list_query_key(arguments: &Value) -> String {
    serde_json::to_string(arguments).unwrap_or_default()
}

/// The dedup check: `Some(stub)` when this exact list payload was already
/// served to this session at the CURRENT registry generation — the model can
/// re-read its earlier result instead of receiving another full catalog dump.
/// Entries recorded at older generations never match (the registry changed;
/// a fresh fetch is correct).
fn list_dedup(
    seen: &[(String, u64)],
    arguments: &Value,
    generation: u64,
) -> Option<trigger::ResultData> {
    let key = list_query_key(arguments);
    if !seen.iter().any(|(k, g)| *k == key && *g == generation) {
        return None;
    }
    let msg = "Unchanged since your earlier engine::functions::list call with these same \
               arguments this session (registry generation unchanged) — re-read that earlier \
               result.";
    Some(trigger::ResultData {
        content: vec![ContentBlock::text(msg.to_string())],
        is_error: false,
        details: json!({ "unchanged": true, "generation": generation }),
    })
}

/// Remember a served list payload (idempotent; oldest evicted past the cap).
fn list_dedup_record(seen: &mut Vec<(String, u64)>, arguments: &Value, generation: u64) {
    let key = list_query_key(arguments);
    if seen.iter().any(|(k, g)| *k == key && *g == generation) {
        return;
    }
    seen.push((key, generation));
    if seen.len() > LIST_QUERIES_MAX {
        seen.remove(0);
    }
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
            for descriptor in snapshot.functions.iter() {
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
    use serde_json::{json, Value};

    #[test]
    fn list_dedup_stubs_only_identical_payload_at_same_generation() {
        let mut seen = Vec::new();
        let args = json!({ "search": "fs" });

        // Never served: no stub; record it.
        assert!(super::list_dedup(&seen, &args, 7).is_none());
        super::list_dedup_record(&mut seen, &args, 7);

        // Identical payload, same generation: stub, not an error.
        let stub = super::list_dedup(&seen, &args, 7).expect("stub");
        assert!(!stub.is_error);
        assert_eq!(stub.details["unchanged"], json!(true));

        // Key order does not defeat the dedup (canonical serialization)...
        let reordered: Value = serde_json::from_str("{\"search\":\"fs\"}").expect("parse");
        assert!(super::list_dedup(&seen, &reordered, 7).is_some());
        // ...but a different filter or a bumped generation dispatches fresh.
        assert!(super::list_dedup(&seen, &json!({ "search": "web" }), 7).is_none());
        assert!(super::list_dedup(&seen, &args, 8).is_none());
    }

    #[test]
    fn list_dedup_record_is_idempotent_and_capped() {
        let mut seen = Vec::new();
        let args = json!({});
        super::list_dedup_record(&mut seen, &args, 1);
        super::list_dedup_record(&mut seen, &args, 1);
        assert_eq!(seen.len(), 1);
        for i in 0..30u64 {
            super::list_dedup_record(&mut seen, &json!({ "prefix": i.to_string() }), 1);
        }
        assert!(seen.len() <= super::LIST_QUERIES_MAX);
        // Oldest evicted first: the original empty payload is gone.
        assert!(super::list_dedup(&seen, &args, 1).is_none());
    }

    #[test]
    fn registry_notice_stamps_silently_then_fires_on_mismatch() {
        // First sighting (None): stamp, no notice.
        assert!(super::registry_notice(None, 7).is_none());
        // Acknowledged generation still current: no notice.
        assert!(super::registry_notice(Some(7), 7).is_none());
        // Registry moved on: notice fires.
        assert!(super::registry_notice(Some(6), 7).is_some());
    }

    #[test]
    fn policy_aid_names_the_narrowed_surface_and_skips_wildcards() {
        use crate::types::turn::FunctionPolicy;
        // No policy / empty allow: dispatch is off entirely — say so.
        assert!(super::policy_aid(None).unwrap().contains("disabled"));
        let empty = FunctionPolicy::default();
        assert!(super::policy_aid(Some(&empty))
            .unwrap()
            .contains("disabled"));
        // A `*` allow is the full surface: the discovery doctrine applies, no aid.
        let full = FunctionPolicy {
            allow: vec!["*".into()],
            ..Default::default()
        };
        assert!(super::policy_aid(Some(&full)).is_none());
        // Narrowed: the exact surface is spelled out, discovery is called out.
        let narrowed = FunctionPolicy {
            allow: vec!["state::set".into(), "state::get".into()],
            deny: vec!["state::delete".into()],
            ..Default::default()
        };
        let aid = super::policy_aid(Some(&narrowed)).unwrap();
        assert!(aid.contains("ONLY these functions: state::get, state::set"));
        assert!(aid.contains("Deny-listed on top: state::delete"));
        assert!(aid.contains("engine::functions::list"));
        // Long allow-lists are capped, not dumped.
        let long = FunctionPolicy {
            allow: (0..40).map(|i| format!("w{i:02}::fn")).collect(),
            ..Default::default()
        };
        let aid = super::policy_aid(Some(&long)).unwrap();
        assert!(aid.contains("w00::fn"));
        assert!(!aid.contains("w39::fn"));
        assert!(aid.contains("…"));
    }

    mod rotate_mid_generation_users {
        use super::super::rotate_mid_generation_users;
        use serde_json::{json, Value};

        fn user(tag: &str) -> Value {
            json!({"role": "user", "content": [{"type": "text", "text": tag}]})
        }
        fn assistant(tag: &str) -> Value {
            json!({"role": "assistant", "content": [{"type": "text", "text": tag}]})
        }
        fn assistant_call(tag: &str) -> Value {
            json!({"role": "assistant", "content": [
                {"type": "text", "text": tag},
                {"type": "function_call", "id": "t1", "function_id": "f", "arguments": {}},
            ]})
        }
        fn result(tag: &str) -> Value {
            json!({"role": "function_result", "function_call_id": "t1", "function_id": "f",
                   "content": [{"type": "text", "text": tag}]})
        }
        fn tags(msgs: &[Value]) -> Vec<&str> {
            msgs.iter()
                .map(|m| {
                    m["content"][0]["text"]
                        .as_str()
                        .or_else(|| m["content"].as_str())
                        .unwrap_or("?")
                })
                .collect()
        }

        #[test]
        fn mid_generation_notification_rotates_past_the_reply() {
            // The live prefill-400 repro: notification appended during
            // generation/assembly sits before the assistant entry; the
            // re-generate must end with the notification, not the assistant.
            let mut m = vec![
                user("task"),
                assistant("a1"),
                user("notif"),
                assistant("a2"),
            ];
            rotate_mid_generation_users(&mut m, 2);
            assert_eq!(tags(&m), vec!["task", "a1", "a2", "notif"]);
        }

        #[test]
        fn opening_message_never_moves() {
            // Suffix covering the whole list (first generate, or compaction
            // cut into the suffix): the window clamps off the opener so the
            // context still starts with a user turn.
            let mut m = vec![user("task"), user("notif"), assistant("a1")];
            rotate_mid_generation_users(&mut m, 3);
            assert_eq!(tags(&m), vec!["task", "a1", "notif"]);
        }

        #[test]
        fn empty_suffix_is_a_noop() {
            let mut m = vec![user("task"), assistant("a1")];
            rotate_mid_generation_users(&mut m, 0);
            assert_eq!(tags(&m), vec!["task", "a1"]);
        }

        #[test]
        fn trailing_assistant_with_calls_is_left_for_the_result_path() {
            // Calls pending → results follow → the wire never ends assistant.
            let mut m = vec![user("task"), user("notif"), assistant_call("a1")];
            rotate_mid_generation_users(&mut m, 2);
            assert_eq!(tags(&m), vec!["task", "notif", "a1"]);
        }

        #[test]
        fn results_in_the_new_suffix_stay_in_place() {
            // Result and notification both landed after the watermark; only
            // the user message rotates, pairing stays intact.
            let mut m = vec![
                user("task"),
                assistant_call("a1"),
                result("r1"),
                user("notif"),
                assistant("a2"),
            ];
            rotate_mid_generation_users(&mut m, 3);
            assert_eq!(tags(&m), vec!["task", "a1", "r1", "a2", "notif"]);
        }

        #[test]
        fn trailing_user_is_a_noop() {
            let mut m = vec![user("task"), assistant("a1"), user("steer")];
            rotate_mid_generation_users(&mut m, 2);
            assert_eq!(tags(&m), vec!["task", "a1", "steer"]);
        }

        #[test]
        fn redelivered_step_with_stale_empty_assistant_still_rotates() {
            // The verifier's retry schedule: attempt 1 appended its empty
            // assistant entry then died before generating; the retry's
            // suffix covers [notif, empty-assistant]. The notification must
            // still rotate past the trailing (empty, call-less) assistant.
            let mut m = vec![
                user("task"),
                assistant("a1"),
                user("notif"),
                json!({"role": "assistant", "content": []}),
            ];
            rotate_mid_generation_users(&mut m, 2);
            assert_eq!(m[3]["role"], "user");
            assert_eq!(m[3]["content"][0]["text"], "notif");
        }
    }

    #[test]
    fn patch_orphaned_calls_injects_elided_results_adjacent_to_the_call() {
        use serde_json::json;
        let mut msgs = vec![
            json!({"role": "user", "content": [{"type": "text", "text": "hi"}]}),
            json!({"role": "assistant", "content": [
                {"type": "function_call", "id": "toolu_ok", "function_id": "a::b", "arguments": {}},
                {"type": "function_call", "id": "toolu_orphan", "function_id": "c::d", "arguments": {}},
            ]}),
            json!({"role": "function_result", "function_call_id": "toolu_ok", "function_id": "a::b",
                   "content": [{"type": "text", "text": "ok"}]}),
            json!({"role": "user", "content": [{"type": "text", "text": "next"}]}),
        ];
        assert_eq!(super::patch_orphaned_calls(&mut msgs), 1);
        // The synthetic result sits directly after the assistant message.
        assert_eq!(
            msgs[2]
                .get("function_call_id")
                .and_then(serde_json::Value::as_str),
            Some("toolu_orphan")
        );
        assert_eq!(msgs.len(), 5);
        // Fully paired context is untouched.
        assert_eq!(super::patch_orphaned_calls(&mut msgs), 0);
        assert_eq!(msgs.len(), 5);
    }

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
