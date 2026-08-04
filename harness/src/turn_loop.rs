//! The durable turn loop (harness.md § The loop). One `harness::turn` step
//! assembles context, generates one assistant message, dispatches any function
//! calls, then either re-enqueues (model reacts to results / steering) or
//! finalises the turn. Steps are at-least-once: the stale-step guard,
//! deterministic entry ids, and per-call checkpoints make redelivery safe.

use async_trait::async_trait;
use iii_helpers::observability::opentelemetry::trace::{Status, TraceContextExt as _};
use iii_helpers::observability::opentelemetry::{Context, KeyValue};
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
pub use crate::queue::TURN_QUEUE;
use crate::trigger;
use crate::types::content::ContentBlock;
use crate::types::event::ErrorKind;
use crate::types::message::{empty_assistant, AgentMessage, AssistantMessage};
use crate::types::model::AgentFunction;
use crate::types::turn::{
    CallCheckpoint, CallState, ExposeMode, FunctionPolicy, TurnRecord, TurnStatus,
};

#[derive(Clone, Copy)]
struct FailureInfo {
    code: &'static str,
    phase: &'static str,
    retryable: bool,
}

const INTERNAL_FAILURE: FailureInfo = FailureInfo {
    code: "harness.turn_internal",
    phase: "execution",
    retryable: false,
};

const CONTEXT_OVERFLOW_FAILURE: FailureInfo = FailureInfo {
    code: "harness.context_overflow",
    phase: "context_assembly",
    retryable: false,
};

const BUDGET_EXCEEDED_FAILURE: FailureInfo = FailureInfo {
    code: "harness.budget_exceeded",
    phase: "budget_preflight",
    retryable: false,
};

const BUDGET_UNAVAILABLE_FAILURE: FailureInfo = FailureInfo {
    code: "harness.budget_unavailable",
    phase: "budget_preflight",
    retryable: false,
};

/// Provider adapters add framing outside the fields visible to the harness.
/// Keep a small fixed reserve in addition to the deterministic JSON estimate.
const PROVIDER_FRAMING_ALLOWANCE_TOKENS: u64 = 64;

/// Assembly headroom reserved when pre-generate hooks are bound: hook
/// appends land AFTER assembly fits the context, so without a reservation
/// any append overflows an exactly-full context and fails the turn.
const PRE_GENERATE_HOOK_ALLOWANCE_TOKENS: u64 = 256;

/// Extra margin folded into the one-shot re-assembly after a post-assembly
/// overflow, covering hook-output variance on the retry.
const REASSEMBLY_HEADROOM_MARGIN_TOKENS: u64 = 256;

fn estimate_request_overhead_tokens(
    response_format: Option<&Value>,
    provider_options: Option<&Value>,
) -> u64 {
    let mut fields = serde_json::Map::new();
    if let Some(response_format) = response_format {
        fields.insert("response_format".to_string(), response_format.clone());
    }
    if let Some(provider_options) = provider_options {
        fields.insert("provider_options".to_string(), provider_options.clone());
    }
    let serialized_chars = Value::Object(fields).to_string().chars().count() as u64;
    let serialized_tokens = serialized_chars.saturating_add(3) / 4;
    PROVIDER_FRAMING_ALLOWANCE_TOKENS.saturating_add(serialized_tokens)
}
/// The enqueued `harness::turn` step payload.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TurnStepPayload {
    pub session_id: String,
    pub turn_id: String,
    pub step: u64,
    /// Preview carried from the turn record so the step can stamp the
    /// `iii.tag.message` baggage before any state read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_preview: Option<String>,
    /// Sub-agent depth carried from the turn record (0 = top-level), so the
    /// step can stamp the `iii.tag.kind` baggage (`harness.turn` /
    /// `harness.subagent`) before any state read. Defaults to 0 so stale
    /// in-flight payloads from before this field existed still classify as
    /// top-level turns.
    #[serde(default)]
    pub depth: u32,
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

/// Bounded in-place retry for the enqueue that follows a persisted step
/// advance: `queue::enqueue` can be briefly unregistered while the queue
/// worker replays its registrations against a restarted engine, and failing
/// the turn in that window (or worse, wedging it Running with the step
/// persisted but never enqueued) turns a recoverable restart into a dead
/// session (iii-hq/workers#507).
const ENQUEUE_ATTEMPTS: u32 = 5;
const ENQUEUE_RETRY_BACKOFF_MS: u64 = 500;

/// Enqueue the next durable loop step onto the dedicated `harness-turn` queue.
pub async fn enqueue_step(
    iii: &IIIClient,
    session_id: &str,
    turn_id: &str,
    step: u64,
    message_preview: Option<&str>,
    depth: u32,
) -> Result<(), HarnessError> {
    let mut payload =
        json!({ "session_id": session_id, "turn_id": turn_id, "step": step, "depth": depth });
    if let Some(preview) = message_preview {
        payload["message_preview"] = json!(preview);
    }
    let mut last_error = String::new();
    for attempt in 1..=ENQUEUE_ATTEMPTS {
        match iii
            .trigger(TriggerRequest {
                function_id: "harness::turn".to_string(),
                payload: payload.clone(),
                action: Some(TriggerAction::Enqueue {
                    queue: TURN_QUEUE.to_string(),
                }),
                timeout_ms: None,
            })
            .await
        {
            Ok(_) => return Ok(()),
            Err(e) => {
                last_error = e.to_string();
                if attempt < ENQUEUE_ATTEMPTS {
                    tracing::warn!(
                        session_id = %session_id,
                        turn_id = %turn_id,
                        step,
                        attempt,
                        error = %last_error,
                        "enqueue harness::turn failed; retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(ENQUEUE_RETRY_BACKOFF_MS))
                        .await;
                }
            }
        }
    }
    Err(HarnessError::Dependency(format!(
        "enqueue harness::turn: {last_error}"
    )))
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
            // The turn record is the authoritative recovery snapshot. A
            // transcript alone cannot recover budgets, parent linkage, output
            // contracts, or dispatch policy safely, so an absent record stays
            // a stale delivery and is acknowledged without fabricating state.
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

    // Deliver messages queued while the previous step streamed: append them in
    // arrival order before the context load, so this generation sees them all
    // at once (harness.md § Concurrency & steering).
    drain_queued(deps, &session, &record.session_id).await?;

    // First-step bookkeeping: mark working + emit turn-started + pre_turn hook.
    // The reason is the live phase detail UIs render while the shimmer shows;
    // it advances to "waiting for <model>" right before the generation RPC.
    let _ = session
        .set_status(&record.session_id, "working", Some("preparing context"))
        .await;
    if payload.step == 0 && record.turn_count == 0 {
        deps.events
            .emit_started(
                &record.session_id,
                &record.turn_id,
                record.parent.as_ref(),
                record.display_parent_session_id.as_deref(),
            )
            .await;
        if let Err(reason) = deps.hooks.run_pre_turn(&record, payload.step).await {
            return finalize_failed(
                deps,
                &session,
                &mut record,
                &format!("pre_turn hook denied: {reason}"),
                FailureInfo {
                    code: "harness.pre_turn_denied",
                    phase: "pre_turn",
                    retryable: false,
                },
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
        // The cap ends the turn but must not BYPASS the post-turn gate: with
        // no steps left to correct anything, a validator that rejects this
        // residue FAILS the turn — a runaway must never complete as if
        // validated (live-caught by the validation_chain e2e: a turn that
        // burned its steps mid-loop finalized "completed" with the goal
        // unmet).
        return match deps.hooks.run_post_turn(&record, record.step, &text).await {
            Ok(()) => finalize_completed(deps, &session, &mut record, Some(text)).await,
            Err(deny) => {
                record.result = Some(text);
                finalize_failed(
                    deps,
                    &session,
                    &mut record,
                    &format!(
                        "max_turns reached with the post-turn gate unsatisfied: {}",
                        deny.reason
                    ),
                    FailureInfo {
                        code: "harness.output_contract_invalid",
                        phase: "output_validation",
                        retryable: false,
                    },
                )
                .await
            }
        };
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

    // Build every deterministic model-facing input before context assembly.
    // Registry-change notice: if the function registry changed since this
    // session last acknowledged its generation, tell the model its cached
    // contracts may be stale. First sighting stamps silently.
    let current_generation = deps.functions().await.generation;
    let mut assembly_system_prompt =
        with_filesystem_root_aid(record.options.system_prompt.clone(), &record);
    if let Some(notice) = registry_notice(record.functions_generation, current_generation) {
        assembly_system_prompt = Some(match assembly_system_prompt.take() {
            Some(prompt) if !prompt.is_empty() => format!("{prompt}\n\n{notice}"),
            _ => notice,
        });
    }
    record.functions_generation = Some(current_generation);

    // Resolve the output-contract strategy and build the invocation surface:
    // the exposure-mode tools plus the synthetic submit_result schema when the
    // contract uses the fallback.
    let strategy = crate::contract::OutputStrategy::resolve(deps, &record).await;
    let mut tools = build_tools(deps, &record).await;
    if let Some(submit) = strategy.submit_result_tool() {
        tools.push(submit);
    }
    let response_format = strategy.response_format();
    let provider_options = record
        .options
        .provider_options
        .as_ref()
        .and_then(|options| serde_json::to_value(options).ok());
    let request_overhead_tokens =
        estimate_request_overhead_tokens(response_format.as_ref(), provider_options.as_ref());

    // Assemble the model-ready context (+ compaction persistence). An
    // impossible fit is a terminal turn outcome, not an unexpected step error.
    //
    // Post-assembly, pre_generate hooks and orphan repair may GROW the
    // request, while assembly fits to the exact usable ceiling — so on a
    // full context even a tiny hook append overflows the final check
    // (observed live: a ~20-token guidance injection over a 220k-usable
    // request killed a 13-step turn). Two defenses, composable: when
    // pre-generate hooks are bound, reserve a small allowance up front;
    // and if the final count still overflows, re-assemble ONCE with the
    // measured deficit folded into the overhead so assembly makes room —
    // failing the turn only when even that is not enough.
    let mut extra_overhead_tokens: u64 = if deps.hooks.pre_generate.is_empty() {
        0
    } else {
        PRE_GENERATE_HOOK_ALLOWANCE_TOKENS
    };
    let mut reassembled = false;
    let (
        gen_system_prompt,
        gen_annotations,
        gen_messages,
        generation_input_tokens,
        generation_max_output_tokens,
    ) = loop {
        let assembled = match assemble_context(
            deps,
            &session,
            &record,
            &entries,
            payload.step,
            prev_watermark.as_deref(),
            ContextAssemblyInputs {
                system_prompt: assembly_system_prompt.clone(),
                tools: &tools,
                request_overhead_tokens: request_overhead_tokens
                    .saturating_add(extra_overhead_tokens),
            },
        )
        .await
        {
            Ok(assembled) => assembled,
            Err(HarnessError::ContextOverflow(reason)) => {
                return finalize_failed(
                    deps,
                    &session,
                    &mut record,
                    &reason,
                    CONTEXT_OVERFLOW_FAILURE,
                )
                .await;
            }
            Err(error) => return Err(error),
        };

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
                    FailureInfo {
                        code: "harness.pre_generate_denied",
                        phase: "pre_generate",
                        retryable: false,
                    },
                )
                .await;
            }
        };
        let hook_appended = !appended.is_empty();
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
                FailureInfo {
                    code: "harness.empty_context",
                    phase: "context_assembly",
                    retryable: false,
                },
            )
            .await;
        }

        // Hooks and orphan repair can change the assembled request. When nothing
        // did — no appended messages, no orphan patches, prompt unchanged — the
        // request IS the assemble output, whose `token_count` already covers
        // messages + prompt + tools + the request overhead, so the second
        // full-payload count-tokens round trip is pure repetition. Re-count only
        // when the request actually diverged. (The re-count compares against the
        // BASE overhead: the extra reservation exists only to make assembly
        // leave room; it is not part of the real request.)
        let request_unchanged = final_request_unchanged(
            hook_appended,
            patched,
            &gen_system_prompt,
            &assembled.system_prompt,
        );
        let final_request_tokens = if request_unchanged {
            assembled.token_count
        } else {
            let final_count = deps
                .context()
                .await
                .count_tokens(crate::clients::context::CountTokensParams {
                    messages: gen_messages.clone(),
                    model_id: record.options.model.clone(),
                    provider: record.options.provider.clone(),
                    system_prompt: gen_system_prompt.clone(),
                    tools: tools.clone(),
                })
                .await
                .map_err(HarnessError::Dependency)?;
            final_count.tokens.saturating_add(request_overhead_tokens)
        };
        if final_request_tokens <= assembled.usable {
            let max_output_tokens = record
                .options
                .max_output_tokens
                .unwrap_or(assembled.effective_max_output_tokens)
                .min(assembled.effective_max_output_tokens);
            let snapshot = build_context_snapshot(
                &record,
                payload.step,
                &assembled,
                final_request_tokens,
                request_overhead_tokens,
            );
            record.context_snapshot = Some(snapshot);
            break (
                gen_system_prompt,
                gen_annotations,
                gen_messages,
                final_request_tokens,
                max_output_tokens,
            );
        }
        if reassembled {
            let reason = format!(
                "final model request exceeds the assembled usable budget: {final_request_tokens} tokens > {} usable (persists after re-assembly with reserved headroom)",
                assembled.usable
            );
            return finalize_failed(
                deps,
                &session,
                &mut record,
                &reason,
                CONTEXT_OVERFLOW_FAILURE,
            )
            .await;
        }
        // One-shot recovery: fold the measured post-assembly additions (plus
        // margin for hook variance on the retry — hooks re-run against the
        // smaller context) into the reservation and re-assemble. The
        // compaction bookkeeping entry id is per (turn, step), so a
        // re-assembled compaction dedupes instead of double-writing.
        reassembled = true;
        let deficit = reassembly_deficit(final_request_tokens, assembled.token_count);
        extra_overhead_tokens = extra_overhead_tokens
            .saturating_add(deficit)
            .saturating_add(REASSEMBLY_HEADROOM_MARGIN_TOKENS);
        tracing::warn!(
            session_id = %record.session_id,
            turn_id = %record.turn_id,
            deficit,
            extra_overhead_tokens,
            "post-assembly additions exceeded the usable budget; re-assembling with reserved headroom"
        );
    };

    // A stop that landed during context assembly is visible through the
    // in-process signal even though its durable write is blocked by this
    // session lock. Do not reserve budget for a call that will never start.
    if deps.cancels.is_fired(&record.turn_id) {
        record.abort = true;
        return finalize_cancelled(deps, &session, &mut record, "cancelled").await;
    }

    let budget_reservation = match crate::budget::reserve(
        deps,
        &record,
        generation_input_tokens,
        generation_max_output_tokens,
    )
    .await?
    {
        crate::budget::ReserveOutcome::Unlimited => None,
        crate::budget::ReserveOutcome::Reserved(reservation) => Some(reservation),
        crate::budget::ReserveOutcome::Rejected(rejection) => {
            let failure = match &rejection {
                crate::budget::BudgetRejection::Exceeded(_) => BUDGET_EXCEEDED_FAILURE,
                crate::budget::BudgetRejection::Unavailable(_) => BUDGET_UNAVAILABLE_FAILURE,
            };
            return finalize_failed(deps, &session, &mut record, rejection.reason(), failure).await;
        }
    };

    let assistant_origin = origin_with(&record.turn_id, &gen_annotations);

    // Generate: append an empty assistant under a deterministic id, stream
    // deltas into it, then write the final message.
    let assistant_id = ids::assistant_entry_id(&record.turn_id, payload.step);
    let provider = record.options.provider.clone().unwrap_or_default();
    let empty = empty_assistant(&provider, &record.options.model);
    if let Err(error) = session
        .append(
            &record.session_id,
            &AgentMessage::Assistant(empty),
            Some(&assistant_id),
            None,
            Some(&assistant_origin),
        )
        .await
    {
        if let Some(reservation) = budget_reservation.as_ref() {
            crate::budget::release(deps, reservation).await?;
        }
        return Err(error);
    }

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
        // Cloned into the request so the originals stay available for the
        // post-generation exact recount below.
        system_prompt: gen_system_prompt.clone(),
        messages: gen_messages,
        tools: tools.clone(),
        response_format,
        // Forward a cap only when the caller set one. `generation_max_output_tokens`
        // is the internal reservation context assembly budgets against; sending it
        // unasked would put the model's own ceiling on every request and let a
        // provider apply a different policy than its default. When the caller DID
        // ask, send the reservation rather than the raw request — it is their value
        // already clamped to the model's effective limit, so the provider is never
        // told it may emit more than we reserved.
        max_output_tokens: record
            .options
            .max_output_tokens
            .map(|_| generation_max_output_tokens),
        thinking_level: record.options.thinking_level,
        provider_options,
    };
    record.stream_request_id = Some(params.request_id.clone());
    if let Err(error) = crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await {
        if let Some(reservation) = budget_reservation.as_ref() {
            crate::budget::release(deps, reservation).await?;
        }
        return Err(error);
    }

    // A stop may land after reservation but before dispatch. Release the
    // unused amount before finalising the cancelled turn.
    if deps.cancels.is_fired(&record.turn_id) {
        if let Some(reservation) = budget_reservation.as_ref() {
            crate::budget::release(deps, reservation).await?;
        }
        record.abort = true;
        return finalize_cancelled(deps, &session, &mut record, "cancelled").await;
    }

    // Release the per-session lock across the generation RPC. The loop writes no
    // turn record between here and the post-generation cancellation check
    // (router::chat streams into the SESSION, not the turn record), so dropping
    // the lock lets a concurrent harness::stop take it and set the durable abort
    // flag — observed THIS step by the re-read below — instead of leaving the
    // stop blocked behind the whole step and a step of tool execution leaking.
    drop(_guard);

    // Phase detail: context is assembled, the provider round-trip starts now.
    // This is the window users actually wait in (provider time-to-first-token).
    let waiting_reason = format!("waiting for {}", record.options.model);
    let _ = session
        .set_status(&record.session_id, "working", Some(&waiting_reason))
        .await;

    let router = deps.router().await;
    // Harness-local cancel backstop: cuts this await even when router::abort
    // was a no-op (registration race / router restart). Level-triggered, so a
    // stop fired before this subscribe is still observed.
    let abort_rx = deps.cancels.watch(&record.turn_id);
    let mut outcome = match router.chat(params, &sink, abort_rx).await {
        Ok(outcome) => outcome,
        Err(error) => {
            // The provider may have consumed the request without returning
            // usage. Charge the full reservation so transport failures cannot
            // bypass a hard budget.
            if let Some(reservation) = budget_reservation.as_ref() {
                crate::budget::reconcile(deps, reservation, None).await?;
            }
            return Err(error);
        }
    };
    if let Some(reservation) = budget_reservation.as_ref() {
        crate::budget::reconcile(deps, reservation, outcome.message.usage.as_ref()).await?;
    }

    // A stop mid-generation truncates tool-call blocks before their arguments
    // stream. An argument-less call can never execute (the turn is cancelled)
    // and renders as an empty generic card — drop it from the persisted
    // partial. Calls with partial arguments are kept: they show what was
    // forming when the user stopped.
    if outcome.message.stop_reason == crate::types::event::StopReason::Aborted {
        outcome.message.content.retain(|b| {
            !matches!(b, ContentBlock::FunctionCall { arguments, .. }
                if arguments.is_null()
                    || arguments.as_str().is_some_and(str::is_empty)
                    || arguments.as_object().is_some_and(serde_json::Map::is_empty))
        });
    }

    // Generation consumed: advance the steering watermark (persisted by the
    // advance()/finalize call that ends this step).
    record.watermark_entry_id = watermark;

    // Stamp the generation's actual usage into the snapshot, replace the
    // estimated categories with provider-exact counts where a counter
    // exists, and store the session's latest copy. Best-effort: accounting
    // must never fail a turn that generated successfully.
    if let Some(snapshot) = record.context_snapshot.as_mut() {
        snapshot.usage = outcome.message.usage.clone();
        crate::context_snapshot::exactify(snapshot, &router, gen_system_prompt.as_deref(), &tools)
            .await;
        if let Err(error) =
            crate::context_snapshot::put(&deps.iii, snapshot, cfg.session_timeout_ms).await
        {
            tracing::warn!(
                session_id = %record.session_id,
                turn_id = %record.turn_id,
                %error,
                "context snapshot store failed"
            );
        }
    }

    // Persist the final assistant message into the streamed entry.
    let _ = session
        .update_message(
            &record.session_id,
            &assistant_id,
            &outcome.message.content,
            outcome.message.usage.as_ref(),
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
        preserve_assistant_partial(&mut record.result, &outcome.message);
        if transient_resume_allowed(
            outcome.message.error_kind,
            record.transient_resumes,
            record.options.max_transient_resumes,
            record.turn_count,
            record.options.max_turns,
        ) {
            let attempt = record.transient_resumes + 1;
            record_recovery_telemetry(&record, &reason, attempt);
            let _ = session
                .append_custom(
                    &record.session_id,
                    "recovery",
                    json!({
                        "status": "recovering",
                        "summary": format!(
                            "Stream interrupted; preserved partial output and resuming ({attempt}/{}).",
                            record.options.max_transient_resumes
                        ),
                        "reason": reason,
                        "phase": "generation",
                        "attempt": attempt,
                        "max_attempts": record.options.max_transient_resumes,
                        "partial_result_available": record.result.is_some(),
                        "timestamp": AgentMessage::now_ms(),
                    }),
                    &ids::transient_recovery_entry_id(
                        &record.turn_id,
                        attempt,
                        "recovering",
                    ),
                    Some(&origin(&record.turn_id)),
                )
                .await;
            let nudge = AgentMessage::user_text(format!(
                "The provider/router ended transiently after streaming the previous partial \
                 response: {reason}. Resume from the transcript and return the complete deliverable, \
                 incorporating the previous partial even if it was already nearly complete. Do not \
                 repeat function calls that already have successful results."
            ));
            let _ = session
                .append(
                    &record.session_id,
                    &nudge,
                    Some(&ids::transient_resume_nudge_entry_id(
                        &record.turn_id,
                        attempt,
                    )),
                    None,
                    Some(&origin(&record.turn_id)),
                )
                .await?;
            record.transient_resumes = attempt;
            return advance(deps, &mut record).await;
        }
        return finalize_failed(
            deps,
            &session,
            &mut record,
            &reason,
            llm_failure_info(outcome.message.error_kind),
        )
        .await;
    }

    if record.transient_resumes > 0 {
        let attempt = record.transient_resumes;
        let _ = session
            .append_custom(
                &record.session_id,
                "recovery",
                json!({
                    "status": "recovered",
                    "summary": format!(
                        "Generation recovered after transient interruption ({attempt}/{}).",
                        record.options.max_transient_resumes
                    ),
                    "phase": "generation",
                    "attempt": attempt,
                    "max_attempts": record.options.max_transient_resumes,
                    "partial_result_available": record.result.is_some(),
                    "timestamp": AgentMessage::now_ms(),
                }),
                &ids::transient_recovery_entry_id(&record.turn_id, attempt, "recovered"),
                Some(&origin(&record.turn_id)),
            )
            .await;
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
            // Cancel check between tool calls. Only the in-process signal can
            // be observed here: this phase holds the session lock, so the
            // durable abort write in harness::stop is blocked behind it by
            // construction. Executed calls are already checkpointed per-call,
            // so finalizing mid-loop loses nothing.
            if deps.cancels.is_fired(&record.turn_id) {
                record.abort = true;
                return finalize_cancelled(deps, &session, &mut record, "cancelled").await;
            }

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

            // A call still named `agent_trigger` here means plan_calls found
            // no resolvable `function` in its arguments (empty/null/
            // unparseable — local models flub JSON args). The wrapper is not
            // an engine function; fail locally with a teachable error instead
            // of the engine's cryptic function_not_found.
            if call.function_id == policy::AGENT_TRIGGER_NAME {
                let data = trigger::wrapper_without_target_result(&call.arguments);
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

            // Provider-degraded arguments: a stream that died or was cut by
            // max_tokens mid-args arrives as a salvaged `"_partial": true`
            // prefix or a raw `{"_raw": …}` evidence object (the router's
            // degraded_arguments). Executing partial intent is worse than
            // failing — the complete-looking leading fields may be missing
            // the constraints the model was still writing.
            if call.arguments.get("_partial").is_some() || call.arguments.get("_raw").is_some() {
                let data = trigger::truncated_arguments_result(&call.function_id, &call.arguments);
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
                deps.hooks.filesystem_boundary(&call.function_id),
            );
            let (eff_args, pre_ann) = match deps
                .hooks
                .run_pre_trigger(
                    &record,
                    payload.step,
                    &call.id,
                    &call.function_id,
                    &trusted_call_args,
                    None,
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
                        deps.hooks.filesystem_boundary(&call.function_id),
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
                crate::hooks::runner::PreTriggerOutcome::Hold {
                    held_by, arguments, ..
                } => {
                    let info = trigger::PendingInfo {
                        pending_timeout_ms: None,
                        held_by: Some(held_by),
                        held_arguments: Some(arguments),
                        child_session_id: None,
                        child_turn_id: None,
                    };
                    checkpoint_pending(&mut record, &call.id, call, &info);
                    crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
                    continue;
                }
            };

            // harness::spawn is fire-and-forget: seed a child and return its
            // ids immediately (never invoke a target, never park). The child's
            // result reaches consumers only through registered triggers/state.
            // Guard failures skip post_trigger.
            if call.function_id == crate::functions::SPAWN_ID {
                let entry_id = ids::function_result_entry_id(&record.turn_id, &call.id);
                let (data, child) = match crate::subagent::spawn_from_turn(
                    deps, &record, &call.id, &eff_args,
                )
                .await
                {
                    Ok(child) => (crate::subagent::spawned_result(&child), Some(child)),
                    Err(data) => (data, None),
                };
                append_function_result(
                    &session,
                    &record,
                    call,
                    &data,
                    &entry_id,
                    &origin(&record.turn_id),
                )
                .await?;
                // Done immediately, but the child ids stay on the checkpoint:
                // they feed the fan-out guard, `harness::status` children, and
                // the stop cascade.
                record.calls.insert(
                    call.id.clone(),
                    CallCheckpoint {
                        state: CallState::Done,
                        function_id: Some(call.function_id.clone()),
                        entry_id: Some(entry_id),
                        child_session_id: child.as_ref().map(|c| c.session_id.clone()),
                        child_turn_id: child.as_ref().map(|c| c.turn_id.clone()),
                        held_by: None,
                        held_arguments: None,
                        pending_timeout_ms: None,
                        pending_at: None,
                    },
                );
                crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
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
                    held_arguments: None,
                    pending_timeout_ms: None,
                    pending_at: None,
                },
            );
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;

            // Single invocation chokepoint: subscription control calls are
            // intercepted (trusted session injected); everything else invokes the
            // target. Then the post_trigger chain runs over the result.
            let raw = crate::functions::subscribe::invoke(
                deps,
                &engine,
                &policy,
                &call.function_id,
                &eff_args,
                &record.session_id,
                Some(crate::functions::subscribe::CallerModel::from_options(
                    &record.options,
                )),
            )
            .await;
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
                        // A post-trigger release re-invokes the target: keep
                        // the fully pre-mutated args, not the model originals.
                        held_arguments: Some(eff_args.clone()),
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
    if has_user_after_watermark(&session, &record).await? || has_queued(deps, &record).await? {
        return advance(deps, &mut record).await;
    }

    finalize_with_contract(deps, &session, &mut record, &strategy, &outcome.message).await
}

/// Whether model-visible messages are parked in the session's queue
/// (harness.md § Concurrency & steering). Custom-role rows never reach the
/// model context, so a custom-only queue must not steer — a re-generate over
/// an assistant-tailed context is a guaranteed provider prefill rejection.
/// The finalize drain still delivers them to the transcript.
async fn has_queued(deps: &Deps, record: &TurnRecord) -> Result<bool, HarnessError> {
    let cfg = deps.cfg().await;
    let rows =
        crate::state::list_queued(&deps.iii, &record.session_id, cfg.session_timeout_ms).await?;
    Ok(rows
        .iter()
        .any(|r| !matches!(r.message, AgentMessage::Custom(_))))
}

/// How many of these parked rows are MODEL-VISIBLE (non-custom). Custom-role
/// rows are transcript-only status notices that never enter the model context,
/// so they neither steer a live turn (`has_queued`) nor, at finalize, warrant
/// waking a fresh one — a re-generate over an assistant-tailed context is a
/// guaranteed provider prefill rejection. A parked notification arrives as a
/// user-role message, so it counts.
fn count_model_visible(rows: &[crate::state::QueuedMessage]) -> usize {
    rows.iter()
        .filter(|r| !matches!(r.message, AgentMessage::Custom(_)))
        .count()
}

/// Drain the session's message queue into the transcript in arrival order,
/// returning how many drained rows were MODEL-VISIBLE. Idempotent: each row
/// appends under its stored deterministic entry id, and rows are deleted only
/// after the append lands — a redelivered step re-drains as a no-op and reports
/// zero, since there is nothing left to drain.
async fn drain_queued(
    deps: &Deps,
    session: &SessionClient,
    session_id: &str,
) -> Result<usize, HarnessError> {
    let cfg = deps.cfg().await;
    let rows = crate::state::list_queued(&deps.iii, session_id, cfg.session_timeout_ms).await?;
    let model_visible = count_model_visible(&rows);
    for row in rows {
        session
            .append(
                session_id,
                &row.message,
                Some(&row.entry_id),
                None,
                row.origin.as_ref(),
            )
            .await?;
        crate::state::delete_queued(&deps.iii, session_id, &row.id, cfg.session_timeout_ms).await?;
    }
    Ok(model_visible)
}

/// Finalize drain: a message that parked after the loop's last in-step queue
/// check still lands in the transcript here. Returns `true` when it delivered a
/// MODEL-VISIBLE message — the signal that the finalizing turn must reseed
/// (via [`reseed_after_finalize_drain`]) so something reacts to it. Without the
/// reseed a parked notification sits unread with no turn to process it, which
/// strands an autonomous run that ended its turn expecting the fire to wake it.
/// Never blocks the finalise.
async fn drain_queued_best_effort(deps: &Deps, session: &SessionClient, session_id: &str) -> bool {
    match drain_queued(deps, session, session_id).await {
        Ok(model_visible) => model_visible > 0,
        Err(e) => {
            tracing::warn!(session_id = %session_id, error = %e, "finalize queue drain failed");
            false
        }
    }
}

/// Seed a fresh turn after a finalize drain delivered a model-visible message
/// with no turn to react to it. Reuses the finalized turn's frozen options
/// (model / provider / dispatch policy / prompt) and last-acked registry
/// generation so the woken turn keeps the agent's capabilities — the same
/// outcome an external `harness::send` produces against a now-terminal session.
///
/// MUST be called AFTER the terminal `put_turn`: the turn slot is keyed per
/// session, so seeding before the finalize write would be clobbered by it. The
/// caller gates on the drain actually delivering a row, so a redelivered
/// finalize (queue at-least-once) drains nothing and does not double-seed; a
/// concurrent external send racing the same slot is resolved by `run_step`'s
/// stale-turn guard, exactly as two racing sends already are.
async fn reseed_after_finalize_drain(deps: &Deps, record: &TurnRecord) {
    let cfg = deps.cfg().await;
    // Carry the finalized turn's lineage onto the reseeded one: it is the same
    // session continuing, so its depth still counts against the spawn budget
    // and its console nesting must not flatten.
    let lineage = crate::functions::send::TurnLineage {
        depth: record.depth,
        parent: record.parent.clone(),
        display_parent_session_id: record.display_parent_session_id.clone(),
    };
    if let Err(e) = crate::functions::send::seed_new(
        deps,
        &cfg,
        &record.session_id,
        record.options.clone(),
        record.functions_generation,
        None,
        &lineage,
    )
    .await
    {
        tracing::warn!(
            session_id = %record.session_id,
            error = %e,
            "reseed after finalize drain failed; a parked notification may be stranded",
        );
    }
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
    let validation = crate::contract::validate_json(&value, strategy.schema());
    // Close the consumed call in the transcript FIRST: an assistant
    // `function_call` with no `function_result` leaves the persisted history
    // malformed, and the next STEERED turn's provider request then aborts
    // mid-stream ("stream ended without a terminal frame" — reproduced on
    // two providers, repro session steer-repro-1). Deterministic entry id →
    // idempotent under step redelivery.
    let (text, is_error) = match &validation {
        Ok(()) => ("result recorded".to_string(), false),
        Err(msg) => (msg.clone(), true),
    };
    let data = crate::trigger::ResultData {
        content: vec![ContentBlock::text(text)],
        is_error,
        details: Value::Null,
    };
    let entry_id = ids::function_result_entry_id(&record.turn_id, &submit.id);
    append_function_result(
        session,
        record,
        submit,
        &data,
        &entry_id,
        &origin(&record.turn_id),
    )
    .await?;
    match validation {
        Ok(()) => complete_validated(deps, session, record, value).await,
        Err(msg) => retry_or_giveup(deps, session, record, &msg, value).await,
    }
}

/// Contract-valid result → run the `post_turn` hook chain (validators
/// attached via `engine::register_trigger` on `harness::hook::post-turn`)
/// before finalising; a deny re-prompts through the same retry budget as a
/// schema failure.
async fn complete_validated(
    deps: &Deps,
    session: &SessionClient,
    record: &mut TurnRecord,
    value: Value,
) -> Result<TurnStepResult, HarnessError> {
    match deps.hooks.run_post_turn(record, record.step, &value).await {
        Ok(()) => finalize_completed(deps, session, record, Some(value)).await,
        Err(deny) => {
            retry_or_giveup_with(deps, session, record, &deny.reason, value, deny.prompt).await
        }
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
            complete_validated(deps, session, record, json!(text)).await
        }
        crate::contract::OutputStrategy::ProviderNativeJson { schema } => {
            match crate::contract::parse_json_text(&text) {
                Ok(value) => match crate::contract::validate_json(&value, schema.as_ref()) {
                    Ok(()) => complete_validated(deps, session, record, value).await,
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
    retry_or_giveup_with(deps, session, record, error, fallback_result, None).await
}

/// `nudge_override`: a custom corrective prompt (a post-turn binding's
/// `retry_prompt`) sent verbatim instead of the generic wrapper. `error`
/// still names the real reason for logs and the giveup failure record.
async fn retry_or_giveup_with(
    deps: &Deps,
    session: &SessionClient,
    record: &mut TurnRecord,
    error: &str,
    fallback_result: Value,
    nudge_override: Option<String>,
) -> Result<TurnStepResult, HarnessError> {
    if record.validation_retries < record.options.max_validation_retries {
        let attempt = record.validation_retries + 1;
        let nudge = AgentMessage::user_text(nudge_override.unwrap_or_else(|| {
            format!(
                "Your previous result was not accepted: {error}. Please provide a corrected \
                 result that satisfies the required output contract."
            )
        }));
        // `validation: true` marks the entry as a validation nudge so the
        // console renders it as machine-authored (like notification/spawn
        // origins) instead of a human-typed message.
        let _ = session
            .append(
                &record.session_id,
                &nudge,
                Some(&ids::validation_nudge_entry_id(&record.turn_id, attempt)),
                None,
                Some(&json!({ "turn_id": record.turn_id, "validation": true })),
            )
            .await?;
        record.validation_retries = attempt;
        advance(deps, record).await
    } else {
        record.result = Some(fallback_result);
        finalize_failed(
            deps,
            session,
            record,
            error,
            FailureInfo {
                code: "harness.output_contract_invalid",
                phase: "output_validation",
                retryable: false,
            },
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
    enqueue_step(
        &deps.iii,
        &record.session_id,
        &record.turn_id,
        next,
        record.message_preview.as_deref(),
        record.depth,
    )
    .await?;
    Ok(TurnStepResult {
        session_id: record.session_id.clone(),
        status: TurnStatus::Running,
        next_step: Some(next),
        skipped: false,
    })
}

/// A completing turn is terminal unless its session still owns an armed wake
/// (a one-shot notify subscription): the wiring turn of a one-way run ends
/// with its `turn_complete` watcher armed and completes non-terminally; the
/// final compose turn (wake consumed, nothing re-armed) is terminal.
/// Consumers finalize a logical exchange only on `terminal: true`.
async fn turn_is_terminal(deps: &Deps, session_id: &str) -> bool {
    !crate::bindings::session_expects_wake(deps, session_id).await
}

async fn finalize_completed(
    deps: &Deps,
    session: &SessionClient,
    record: &mut TurnRecord,
    result: Option<Value>,
) -> Result<TurnStepResult, HarnessError> {
    let woke = drain_queued_best_effort(deps, session, &record.session_id).await;
    let cfg = deps.cfg().await;
    record.status = TurnStatus::Completed;
    record.result = result.clone();
    record.result_error = None;
    record.updated_at = AgentMessage::now_ms();
    crate::state::put_turn(&deps.iii, record, cfg.session_timeout_ms).await?;
    deps.cancels.clear(&record.turn_id);
    let _ = session.set_status(&record.session_id, "done", None).await;
    deps.events
        .emit_completed(
            &record.session_id,
            &record.turn_id,
            "completed",
            result.as_ref(),
            None,
            None,
            record.parent.as_ref(),
            record.display_parent_session_id.as_deref(),
            turn_is_terminal(deps, &record.session_id).await,
            record.context_snapshot.as_ref(),
        )
        .await;
    // Sub-agent turns resolve the parent's pending call with their result.
    if let Some(parent) = record.parent.clone() {
        crate::deferred::resolve_parent(deps, &parent, "completed", result.as_ref(), None).await;
    }
    // Second sweep, AFTER the terminal write, pairing with `try_enqueue`'s
    // post-enqueue recheck: a send whose recheck still saw `Running` must have
    // enqueued before the terminal write landed, so this sweep collects its
    // row; a recheck that sees the terminal record seeds its own turn. Without
    // it, a row enqueued between the first drain and the terminal write would
    // strand — queued against a turn that will never drain again.
    let woke = woke || drain_queued_best_effort(deps, session, &record.session_id).await;
    // A message parked during this turn's final step was just drained to the
    // transcript with no turn to react to it; seed one now (after the terminal
    // write above, or it would clobber the fresh turn's slot).
    if woke {
        reseed_after_finalize_drain(deps, record).await;
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
    failure: FailureInfo,
) -> Result<TurnStepResult, HarnessError> {
    let woke = drain_queued_best_effort(deps, session, &record.session_id).await;
    let cfg = deps.cfg().await;
    record.status = TurnStatus::Failed;
    record.result_error = Some(reason.to_string());
    record.updated_at = AgentMessage::now_ms();
    record_failure_telemetry(record, reason, failure);
    crate::state::put_turn(&deps.iii, record, cfg.session_timeout_ms).await?;
    deps.cancels.clear(&record.turn_id);
    let _ = session
        .append_custom(
            &record.session_id,
            "error",
            json!({
                "status": "error",
                "summary": reason,
                "reason": reason,
                "code": failure.code,
                "message": reason,
                "retryable": failure.retryable,
                "phase": failure.phase,
                "partial_result_available": record.result.is_some(),
                "recovery": {
                    "attempted": record.transient_resumes,
                    "max_attempts": record.options.max_transient_resumes,
                    "outcome": if record.transient_resumes > 0 { "exhausted" } else { "not_attempted" },
                },
                "next_actions": [
                    "inspect the failure reason",
                    "retry only after correcting the dependency or request"
                ],
                "artifacts": {
                    "session_id": record.session_id,
                    "turn_id": record.turn_id,
                },
                "timestamp": AgentMessage::now_ms(),
            }),
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
            record.result.as_ref(),
            Some(reason),
            Some(reason),
            record.parent.as_ref(),
            record.display_parent_session_id.as_deref(),
            turn_is_terminal(deps, &record.session_id).await,
            record.context_snapshot.as_ref(),
        )
        .await;
    if let Some(parent) = record.parent.clone() {
        // Settle any parked parent call. Fire-and-forget spawns settled `Done`
        // at spawn time, so this usually no-ops — and that is the whole story:
        // a child's failure reaches its parent only through the medium the
        // child was told to write (an `error` status it managed to record) or
        // through the parent's own deadlines (a `timer` wake, a binding
        // `lifecycle`, a `harness::status` poll). Nothing is injected into the
        // parent session on a child's behalf.
        crate::deferred::resolve_parent(
            deps,
            &parent,
            "failed",
            record.result.as_ref(),
            Some(reason),
        )
        .await;
    }
    // Second post-terminal sweep, as in `finalize_completed`: closes the
    // enqueue-after-drain window against `try_enqueue`'s recheck.
    let woke = woke || drain_queued_best_effort(deps, session, &record.session_id).await;
    // As in `finalize_completed`: a message that parked during the failing
    // turn's final step is genuine new input (a notification, a steer) and
    // deserves a turn, the same as an external send arriving at a failed
    // session. Gated on the drain, so it cannot loop on the failure itself.
    if woke {
        reseed_after_finalize_drain(deps, record).await;
    }
    Ok(TurnStepResult {
        session_id: record.session_id.clone(),
        status: TurnStatus::Failed,
        next_step: None,
        skipped: false,
    })
}

fn llm_failure_info(error_kind: Option<ErrorKind>) -> FailureInfo {
    match error_kind {
        Some(ErrorKind::AuthExpired) => FailureInfo {
            code: "llm.auth_expired",
            phase: "generation",
            retryable: false,
        },
        Some(ErrorKind::RateLimited) => FailureInfo {
            code: "llm.rate_limited",
            phase: "generation",
            retryable: true,
        },
        Some(ErrorKind::ContextOverflow) => FailureInfo {
            code: "llm.context_overflow",
            phase: "generation",
            retryable: false,
        },
        Some(ErrorKind::Transient) => FailureInfo {
            code: "llm.transient",
            phase: "generation",
            retryable: true,
        },
        Some(ErrorKind::Permanent) => FailureInfo {
            code: "llm.permanent",
            phase: "generation",
            retryable: false,
        },
        None => FailureInfo {
            code: "llm.generation_failed",
            phase: "generation",
            retryable: false,
        },
    }
}

fn record_recovery_telemetry(record: &TurnRecord, reason: &str, attempt: u32) {
    let cx = Context::current();
    let span = cx.span();
    if !span.span_context().is_valid() {
        return;
    }
    span.set_attribute(KeyValue::new("iii.turn.recovery_attempt", attempt as i64));
    span.set_attribute(KeyValue::new(
        "iii.turn.recovery_max",
        record.options.max_transient_resumes as i64,
    ));
    span.set_attribute(KeyValue::new(
        "iii.turn.partial_result_available",
        record.result.is_some(),
    ));
    span.add_event(
        "harness.turn.recovery",
        vec![
            KeyValue::new("attempt", attempt as i64),
            KeyValue::new("max_attempts", record.options.max_transient_resumes as i64),
            KeyValue::new("reason", reason.to_string()),
        ],
    );
}

fn record_failure_telemetry(record: &TurnRecord, reason: &str, failure: FailureInfo) {
    let cx = Context::current();
    let span = cx.span();
    if !span.span_context().is_valid() {
        return;
    }
    span.set_attribute(KeyValue::new("error.type", failure.code));
    span.set_attribute(KeyValue::new("error.message", reason.to_string()));
    span.set_attribute(KeyValue::new("iii.turn.failure_phase", failure.phase));
    span.set_attribute(KeyValue::new("iii.turn.retryable", failure.retryable));
    span.set_attribute(KeyValue::new(
        "iii.turn.partial_result_available",
        record.result.is_some(),
    ));
    span.set_attribute(KeyValue::new(
        "iii.turn.transient_resumes",
        record.transient_resumes as i64,
    ));
    span.set_attribute(KeyValue::new(
        "iii.turn.max_transient_resumes",
        record.options.max_transient_resumes as i64,
    ));
    span.set_attribute(KeyValue::new("iii.tag.outcome", "failed"));
    span.set_status(Status::error(reason.to_string()));
}

/// Keep a streamed partial available to failure observers without turning it
/// into a successful turn result. Text stays ergonomic; non-text blocks retain
/// their typed wire shape.
fn assistant_partial_result(message: &AssistantMessage) -> Option<Value> {
    if message.content.is_empty() {
        return None;
    }
    let text = ContentBlock::join_text(&message.content);
    if !text.trim().is_empty() {
        Some(json!(text))
    } else {
        serde_json::to_value(&message.content).ok()
    }
}

fn preserve_assistant_partial(current: &mut Option<Value>, message: &AssistantMessage) {
    if let Some(partial) = assistant_partial_result(message) {
        *current = Some(partial);
    }
}

fn transient_resume_allowed(
    error_kind: Option<crate::types::event::ErrorKind>,
    resumes: u32,
    max_resumes: u32,
    turn_count: u32,
    max_turns: u32,
) -> bool {
    error_kind.is_some_and(|kind| kind.is_retryable())
        && resumes < max_resumes
        && turn_count < max_turns
}

async fn finalize_cancelled(
    deps: &Deps,
    session: &SessionClient,
    record: &mut TurnRecord,
    reason: &str,
) -> Result<TurnStepResult, HarnessError> {
    // Deliver any parked rows to the transcript but do NOT reseed: the user
    // stopped this turn, so a parked notification waits for the next explicit
    // send rather than auto-waking a turn they just cancelled.
    let _ = drain_queued_best_effort(deps, session, &record.session_id).await;
    let cfg = deps.cfg().await;
    record.status = TurnStatus::Cancelled;
    record.updated_at = AgentMessage::now_ms();
    crate::state::put_turn(&deps.iii, record, cfg.session_timeout_ms).await?;
    deps.cancels.clear(&record.turn_id);
    // Durable stop marker: without it the transcript just ends mid-thought
    // after a reload. Deterministic id so the live `stop-reason` notice
    // (translate.ts) dedupes against this entry, mirroring `e_{turn}_error`.
    let _ = session
        .append_custom(
            &record.session_id,
            "notice",
            json!({ "reason": "stopped", "message": "stopped by user." }),
            &format!("e_{}_stopped", record.turn_id),
            Some(&origin(&record.turn_id)),
        )
        .await;
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
            turn_is_terminal(deps, &record.session_id).await,
            record.context_snapshot.as_ref(),
        )
        .await;
    if let Some(parent) = record.parent.clone() {
        crate::deferred::resolve_parent(deps, &parent, "cancelled", None, Some(reason)).await;
    }
    // Second post-terminal sweep (see `finalize_completed`): a row enqueued
    // between the first drain and the terminal write still reaches the
    // transcript. Still no reseed — the user cancelled.
    let _ = drain_queued_best_effort(deps, session, &record.session_id).await;
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
    // Serialize with stop/resolve/sweep like every other turn-record writer
    // (locks.rs). Safe: the only caller runs after run_step returned, so its
    // guard is gone. Lock-free, this finalize could interleave with
    // harness::stop's under-lock "stopping" ack and strand the session status.
    let _guard = deps.locks.guard(session_id).await;
    let record = crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms)
        .await
        .ok()
        .flatten();
    match record {
        Some(mut rec) if rec.turn_id == turn_id && !rec.status.is_terminal() => {
            finalize_failed(deps, &session, &mut rec, reason, INTERNAL_FAILURE)
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
            held_arguments: info.held_arguments.clone(),
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
            held_arguments: None,
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
    // otherwise be filtered off the path and the delta fetch would error.
    // The incremental fetch returns only post-watermark entries; when the
    // watermark left the active path (fork), fall back to the full scan.
    if let Some(suffix) = session
        .messages_after(&record.session_id, watermark, true)
        .await?
    {
        return Ok(suffix
            .iter()
            .any(|entry| matches!(&entry.message, Some(AgentMessage::User(_)))));
    }
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
/// candidate window to its tail, and call required `context::assemble`,
/// persisting a new summary when it compacts.
async fn assemble_context(
    deps: &Deps,
    session: &SessionClient,
    record: &TurnRecord,
    entries: &[LoadedEntry],
    step: u64,
    prev_watermark: Option<&str>,
    inputs: ContextAssemblyInputs<'_>,
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
        messages: candidate_values,
        model_id: record.options.model.clone(),
        provider: record.options.provider.clone(),
        system_prompt: inputs.system_prompt,
        previous_summary,
        lease_key: record.session_id.clone(),
        thinking_level: record.options.thinking_level,
        tools: inputs.tools.to_vec(),
        request_overhead_tokens: inputs.request_overhead_tokens,
    };

    let out = match context.assemble(params).await {
        Ok(out) => out,
        Err(error) if is_context_overflow_error(&error) => {
            return Err(HarnessError::ContextOverflow(error));
        }
        Err(error) => return Err(HarnessError::Dependency(error)),
    };

    if out.messages.is_empty() {
        return Err(HarnessError::ContextOverflow(
            "context::assemble returned an empty model-facing context".to_string(),
        ));
    }
    // `context::assemble.token_count` already includes tools and the supplied
    // request overhead; reject any response that violates its hard contract.
    if out.token_count > out.usable {
        return Err(HarnessError::ContextOverflow(format!(
            "context::assemble returned an over-budget context: {} tokens > {} usable",
            out.token_count, out.usable
        )));
    }

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
                "tokens_before": out.applied.tokens_before,
                "summarized_head_tokens": out.applied.summarized_head_tokens,
                "initial_token_count": out.applied.initial_token_count,
                "assembled_token_count": out.token_count,
                "usable_input_tokens": out.usable,
                "effective_max_output_tokens": out.effective_max_output_tokens,
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
    rotate_mid_generation_users(&mut messages, new_suffix_len);
    // Rotation only reorders; the estimator is an order-independent
    // per-message sum, so assemble's count still describes this list.
    Ok(Assembled {
        system_prompt: Some(out.system_prompt),
        messages,
        usable: out.usable,
        token_count: out.token_count,
        effective_max_output_tokens: out.effective_max_output_tokens,
        applied: out.applied,
        breakdown: out.breakdown,
    })
}

fn is_context_overflow_error(error: &str) -> bool {
    error.contains("context/overflow:")
}

/// Fold the assembly the loop already performed into the session's context
/// snapshot. `final_request_tokens >= assembled.token_count` when hooks or
/// orphan repair grew the request; the difference is the hook_guidance
/// category. No counting round trips happen here.
fn build_context_snapshot(
    record: &TurnRecord,
    step: u64,
    assembled: &Assembled,
    final_request_tokens: u64,
    request_overhead_tokens: u64,
) -> crate::context_snapshot::ContextSnapshotV1 {
    use crate::context_snapshot::{ContextSnapshotV1, SnapshotCategoriesV1};
    // A context-manager that predates the breakdown response reports nothing,
    // which is what the all-zero default says.
    let b = assembled.breakdown.clone().unwrap_or_default();
    ContextSnapshotV1 {
        session_id: record.session_id.clone(),
        turn_id: record.turn_id.clone(),
        step,
        model: record.options.model.clone(),
        provider: record.options.provider.clone(),
        estimator: b.estimator,
        usable: assembled.usable,
        effective_max_output_tokens: assembled.effective_max_output_tokens,
        total: final_request_tokens,
        free: assembled.usable.saturating_sub(final_request_tokens),
        categories: SnapshotCategoriesV1 {
            system_prompt: b.system_prompt_tokens,
            tools: b.tools_tokens,
            messages: b.by_role.into(),
            overhead: request_overhead_tokens,
            hook_guidance: final_request_tokens.saturating_sub(assembled.token_count),
        },
        compacted: assembled.applied.compacted,
        summarized_head_tokens: assembled.applied.summarized_head_tokens,
        usage: None,
        timestamp: AgentMessage::now_ms(),
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
        "Your dispatch policy allows ONLY these functions: {shown}{ellipsis}.{deny} This \
         narrowed-policy instruction OVERRIDES the general discovery requirement for this turn: \
         call the listed target ids directly when the task already supplies their arguments. \
         Anything else — including discovery (engine::functions::list / ::info) unless listed \
         above — is denied. Do not probe: if the task genuinely needs an unlisted function or an \
         unknown contract, report that blocker and finish."
    ))
}

struct Assembled {
    system_prompt: Option<String>,
    messages: Vec<Value>,
    usable: u64,
    /// `context::assemble`'s estimate of this exact context (messages +
    /// prompt + tools + request overhead). Reused as the final request
    /// count when nothing mutates the request after assembly.
    token_count: u64,
    /// Model/output ceiling resolved by context-manager for this request.
    effective_max_output_tokens: u64,
    /// What context-manager did to fit the window (compaction and its
    /// bookkeeping), carried whole for the snapshot.
    applied: crate::clients::context::Applied,
    breakdown: Option<crate::clients::context::AssembleBreakdown>,
}

struct ContextAssemblyInputs<'a> {
    system_prompt: Option<String>,
    tools: &'a [AgentFunction],
    request_overhead_tokens: u64,
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
/// Whether the final model request is exactly the assemble output — no
/// hook-appended messages, no orphan patches, prompt unchanged — so
/// `context::assemble.token_count` already describes it and the second
/// count-tokens round trip would be pure repetition.
fn final_request_unchanged(
    hook_appended: bool,
    patched: usize,
    gen_system_prompt: &Option<String>,
    assembled_system_prompt: &Option<String>,
) -> bool {
    !hook_appended && patched == 0 && gen_system_prompt == assembled_system_prompt
}

/// The reservation fold for the one-shot re-assembly: everything the final
/// request grew beyond what assembly believed it built (`token_count`), NOT
/// merely the overshoot past `usable`. Post-assembly additions can dwarf both
/// the up-front allowance and the ceiling overshoot (fp::inject-guidance
/// appends ~2.5k tokens vs the 256-token allowance); folding only the
/// overshoot can leave assembly under its own ceiling, so it rebuilds the
/// identical request and the turn dies terminally on the same count.
fn reassembly_deficit(final_request_tokens: u64, believed_token_count: u64) -> u64 {
    final_request_tokens.saturating_sub(believed_token_count)
}

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
const REGISTRY_CHANGED_NOTICE: &str = "NOTE: the function registry changed during this conversation. Function contracts fetched earlier may be stale — re-fetch the contracts you rely on (engine::functions::info) before calling those functions again.";

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
            // Subscription controls are harness-intercepted virtual functions,
            // so the engine's public registry intentionally does not list
            // them. Publish their real schemas alongside registry functions
            // whenever this turn's dispatch policy allows them.
            let mut tools = crate::functions::subscribe::native_control_tools(&policy);
            for descriptor in snapshot.functions.iter() {
                if !policy.allows(&descriptor.function_id) {
                    continue;
                }
                if tools.iter().any(|tool| tool.name == descriptor.function_id) {
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
                message.usage.as_ref(),
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
    use super::{cancel_requested, count_model_visible, transient_resume_allowed};
    use crate::types::content::ContentBlock;
    use crate::types::event::{ErrorKind, StopReason};
    use crate::types::message::{AgentMessage, CustomMessage, CustomRoleTag};

    fn queued(message: AgentMessage) -> crate::state::QueuedMessage {
        crate::state::QueuedMessage {
            id: "q".into(),
            session_id: "s_1".into(),
            message,
            entry_id: "e".into(),
            origin: None,
            queued_at: 0,
        }
    }

    fn custom_notice(text: &str) -> AgentMessage {
        AgentMessage::Custom(CustomMessage {
            role: CustomRoleTag::Custom,
            custom_type: "notice".into(),
            content: vec![ContentBlock::text(text)],
            display: None,
            details: None,
            timestamp: 0,
        })
    }

    /// The gate that fixes the "notification parked during a turn's final step
    /// is stranded" bug: `finalize_completed`/`finalize_failed` reseed a turn
    /// only when the finalize drain delivered a MODEL-VISIBLE message. A
    /// notification arrives as a user-role message, so it counts and wakes a
    /// turn; a custom-role status notice drains to the transcript but must not
    /// reseed (a re-generate over an assistant-tailed context would be a
    /// provider prefill rejection). A redelivered finalize drains nothing, so
    /// it reports zero and cannot double-seed.
    #[test]
    fn finalize_reseed_gate_counts_only_model_visible_rows() {
        assert_eq!(count_model_visible(&[]), 0, "empty queue never reseeds");

        let notice = queued(custom_notice("scanning…"));
        assert_eq!(
            count_model_visible(std::slice::from_ref(&notice)),
            0,
            "a custom-only queue drains but must not reseed",
        );

        let notification = queued(AgentMessage::user_text("[notification] chunk-done"));
        assert_eq!(
            count_model_visible(std::slice::from_ref(&notification)),
            1,
            "a parked notification (user role) reseeds so the agent reacts",
        );

        let steer = queued(AgentMessage::user_text("also check the tests"));
        assert_eq!(
            count_model_visible(&[notification, notice, steer]),
            2,
            "only the model-visible rows gate the reseed",
        );
    }

    #[test]
    fn request_overhead_always_reserves_provider_framing() {
        assert_eq!(
            super::estimate_request_overhead_tokens(None, None),
            super::PROVIDER_FRAMING_ALLOWANCE_TOKENS + 1
        );
    }

    #[test]
    fn request_overhead_counts_serialized_optional_fields_with_ceiling() {
        let response_format = serde_json::json!({ "type": "json" });
        let provider_options = serde_json::json!({ "temperature": 0.25 });
        let serialized =
            r#"{"provider_options":{"temperature":0.25},"response_format":{"type":"json"}}"#;
        let serialized_tokens = (serialized.chars().count() as u64).div_ceil(4);
        assert_eq!(
            super::estimate_request_overhead_tokens(
                Some(&response_format),
                Some(&provider_options)
            ),
            super::PROVIDER_FRAMING_ALLOWANCE_TOKENS + serialized_tokens
        );
    }

    #[test]
    fn final_count_is_skipped_only_when_nothing_mutated_the_request() {
        let prompt = Some("base".to_string());
        // Untouched request: assemble's token_count is authoritative.
        assert!(super::final_request_unchanged(false, 0, &prompt, &prompt));
        // Any mutation forces the re-count: hook append, orphan patch,
        // or a hook-rewritten system prompt.
        assert!(!super::final_request_unchanged(true, 0, &prompt, &prompt));
        assert!(!super::final_request_unchanged(false, 1, &prompt, &prompt));
        assert!(!super::final_request_unchanged(
            false,
            0,
            &Some("hooked".to_string()),
            &prompt
        ));
        assert!(!super::final_request_unchanged(false, 0, &None, &prompt));
    }

    #[test]
    fn reassembly_deficit_covers_post_assembly_additions_not_just_the_overshoot() {
        // Live incident (2026-07-21, session scan-x7k2-c11-l2): assembly
        // measured ~146_050 under a 148_000 ceiling, the guidance hook
        // appended ~2_552 tokens, final count 148_602. The old
        // ceiling-relative fold (148_602 - 148_000 = 602) left re-assembly
        // a no-op — assembly stayed under its own ceiling, rebuilt the
        // identical request, and the turn died terminally on the same
        // count. The believed-relative fold reserves the full addition.
        assert_eq!(super::reassembly_deficit(148_602, 146_050), 2_552);
        assert!(super::reassembly_deficit(148_602, 146_050) > 148_602 - 148_000);
        // Saturates if the final request came out smaller than believed.
        assert_eq!(super::reassembly_deficit(100, 200), 0);
    }

    #[test]
    fn context_overflow_classification_requires_the_stable_context_code() {
        assert!(super::is_context_overflow_error(
            "context::assemble: handler failed: context/overflow: assembled context requires 101 tokens but usable budget is 100"
        ));
        assert!(!super::is_context_overflow_error(
            "context::assemble: context/model_unresolved: could not resolve model limits"
        ));
        assert!(!super::is_context_overflow_error(
            "context::assemble: request overflowed while parsing"
        ));
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
        assert!(aid.contains("OVERRIDES the general discovery requirement"));
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

    #[test]
    fn transient_resume_is_bounded_and_only_for_retryable_midstream_failures() {
        assert!(transient_resume_allowed(
            Some(ErrorKind::Transient),
            0,
            1,
            1,
            16
        ));
        assert!(transient_resume_allowed(
            Some(ErrorKind::RateLimited),
            0,
            1,
            1,
            16
        ));
        assert!(!transient_resume_allowed(
            Some(ErrorKind::Permanent),
            0,
            1,
            1,
            16
        ));
        assert!(!transient_resume_allowed(
            Some(ErrorKind::Transient),
            1,
            1,
            1,
            16
        ));
        assert!(!transient_resume_allowed(
            Some(ErrorKind::Transient),
            0,
            1,
            16,
            16
        ));
    }

    #[test]
    fn failed_stream_preserves_text_partial_as_observer_result() {
        let mut message = crate::types::message::empty_assistant("openai-codex", "gpt-5");
        message.content = vec![ContentBlock::text("usable partial")];
        assert_eq!(
            super::assistant_partial_result(&message),
            Some(serde_json::json!("usable partial"))
        );
        message.content.clear();
        assert_eq!(super::assistant_partial_result(&message), None);

        let mut preserved = Some(serde_json::json!("first partial"));
        super::preserve_assistant_partial(&mut preserved, &message);
        assert_eq!(preserved, Some(serde_json::json!("first partial")));
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
