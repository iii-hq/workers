//! `context::assemble` — build the model-ready context from a history
//! (context-manager.md § context::assemble). The pipeline, in order:
//! media-normalize -> cap results (always) -> (past a third of the
//! window, or 80 % of the compaction budget) let the provider's count set
//! the budget -> (if over the compaction threshold) age-prune -> (if still
//! over it) compact -> (if over `usable`) emergency-reduce function
//! results -> assemble the final list or return a structured overflow.
//!
//! Two thresholds (D160): the compaction threshold, `min(usable,
//! options.compaction_budget)`, is soft — prune and compaction start past
//! it; `usable` is hard — emergency reduction and the overflow error hold
//! to it alone, so a busy or failed summariser at a low budget never drops
//! content the model could still read.
//!
//! Structural guarantees: `role: "custom"` messages never reach the
//! model-facing list (nor the count); `applied.tail_start_index`
//! indexes the *request* messages array so callers can map it onto
//! their own storage; a busy lease or failed summariser falls through
//! to emergency reduction and the hard budget check.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::summarize::{PromptSteering, summarize_head};
use crate::core::budget::{default_reserved, preserve_recent_budget, usable};
use crate::core::estimate::{Estimator, by_role_from_sizes, estimator_for_model};
use crate::core::lease;
use crate::core::prune::{
    PruneParams, cap_results_with_sizes, emergency_reduce_with_sizes, prune_with_sizes,
};
use crate::core::selection::select;
use crate::core::summary::{RenderedSummary, render_system_prompt, render_system_prompt_with_span};
use crate::error::ContextError;
use crate::functions::resolve_model;
use crate::ports::{Deps, TurnKnobs};
use crate::types::{
    AgentFunction, AgentMessage, ByRoleTokens, ContentBlock, EstimatorName, Failure, ModelInput,
    ProviderOptions, Role, ThinkingLevel,
};

const TEXT_ONLY_IMAGE_OMITTED: &str = "[image omitted: model does not support vision]";
const TOOL_IMAGE_OMITTED: &str =
    "[tool image omitted after use; re-call the function if visual details are needed]";
const USER_IMAGE_OMITTED: &str =
    "[user image omitted after use; ask the user to resend it if visual details are needed]";

fn replace_images(
    blocks: &mut [ContentBlock],
    direct_marker: Option<&str>,
    nested_tool_marker: Option<&str>,
) {
    for block in blocks {
        match block {
            ContentBlock::Image { .. } => {
                if let Some(marker) = direct_marker {
                    *block = ContentBlock::Text {
                        text: marker.into(),
                    };
                }
            }
            ContentBlock::FunctionResult { content, .. } => {
                replace_images(content, nested_tool_marker, nested_tool_marker);
            }
            _ => {}
        }
    }
}

fn normalize_media(messages: &mut [AgentMessage], supports_vision: Option<bool>) {
    let Some(supports_vision) = supports_vision else {
        return;
    };
    let mut later_user = false;
    let mut later_assistant = false;
    let mut later_user_after_assistant = false;

    for message in messages.iter_mut().rev() {
        let direct_marker = if supports_vision {
            match message.role() {
                Role::User if later_user_after_assistant => Some(USER_IMAGE_OMITTED),
                Role::FunctionResult if later_assistant => Some(TOOL_IMAGE_OMITTED),
                _ => None,
            }
        } else {
            Some(TEXT_ONLY_IMAGE_OMITTED)
        };
        let nested_tool_marker = if supports_vision {
            later_assistant.then_some(TOOL_IMAGE_OMITTED)
        } else {
            Some(TEXT_ONLY_IMAGE_OMITTED)
        };

        replace_images(message.content_mut(), direct_marker, nested_tool_marker);

        match message.role() {
            Role::User if !message.has_function_result_block() => later_user = true,
            Role::Assistant => {
                later_assistant = true;
                later_user_after_assistant |= later_user;
            }
            _ => {}
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct AssembleOptions {
    /// Read-only model-switch preview: apply normal media/cap/prune rules and
    /// return the pre-compaction estimate, even when over budget. Never invokes
    /// the summarizer, acquires a lease, or performs emergency reduction.
    #[serde(default)]
    pub preview_only: bool,
    /// Override the default reserve (`min(20000, 10% of context_window)`).
    #[serde(default)]
    pub reserved_tokens: Option<u64>,
    /// user+assistant pairs always kept verbatim (default 2).
    #[serde(default)]
    pub tail_turns: Option<usize>,
    /// Default true.
    #[serde(default)]
    pub allow_compaction: Option<bool>,
    /// Default true.
    #[serde(default)]
    pub allow_prune: Option<bool>,
    /// `function_id`s exempt from the normal prune pass. Emergency
    /// safety reduction may still replace their oversized results.
    #[serde(default)]
    pub protected_functions: Option<Vec<String>>,
    /// Per-result cap override for this call; `null` uses the worker
    /// config (default 20000), `0` disables the cap pass.
    #[serde(default)]
    pub max_result_tokens: Option<u64>,
    /// Estimated tokens for final provider request fields and framing
    /// not otherwise represented by the prompt, messages, or tools.
    #[serde(default)]
    pub request_overhead_tokens: Option<u64>,
    /// Reserve the model's thinking budget for this tier. Also forwarded to
    /// the provider's count, whose template may render it.
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
    /// The `response_format` the turn sends on `router::chat`, forwarded to
    /// the provider's count for providers that render it into the prompt.
    #[serde(default)]
    pub response_format: Option<serde_json::Value>,
    /// Compaction mutual-exclusion key (e.g. a session id); default:
    /// hash of the message set.
    #[serde(default)]
    pub lease_key: Option<String>,
    /// Persisted summary from a prior compaction (see the spec's "The
    /// compaction round trip"); rendered into the system prompt and
    /// used as the anchor if compaction runs again.
    #[serde(default)]
    pub previous_summary: Option<String>,
    /// Also return `input_indices` and `summary_origin` (default false). For
    /// a byte-level generation witness only; ordinary callers leave it off.
    #[serde(default)]
    pub provenance: bool,
    /// Soft compaction threshold, in tokens: prune and compaction start once
    /// the request passes `min(usable, compaction_budget)`, and the
    /// provider's count from `min(usable / 3, 0.8 × compaction_budget)`.
    /// Emergency reduction and the `context/overflow` error still hold to
    /// `usable` alone. Echoed as the response's `compaction_budget`; absent,
    /// compaction starts at `usable` as before.
    #[serde(default)]
    pub compaction_budget: Option<u64>,
    /// Guidance for an inline compaction's summariser, as
    /// `context::compact`'s `instructions`: trimmed, ignored when blank, cut
    /// past 2000 chars.
    #[serde(default)]
    pub compaction_instructions: Option<String>,
    /// `router::chat` `provider_options`, namespaced by provider id (e.g.
    /// the conversation's account under `claude-code`), forwarded verbatim
    /// to every summariser call this assembly makes.
    #[serde(default)]
    pub provider_options: Option<ProviderOptions>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AssembleRequest {
    /// Full candidate history, oldest first.
    pub messages: Option<Vec<AgentMessage>>,
    pub model: ModelInput,
    /// Base system prompt to prepend/merge.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Model-facing invocation schemas included in every budget count.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Named auxiliary texts to count individually with the same
    /// estimator. Counted in `breakdown.by_part` only; never added to
    /// the assembled context.
    #[serde(default)]
    pub parts: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub options: Option<AssembleOptions>,
}

/// How the model limits were resolved.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelResolvedWire {
    Inline,
    Router,
    Fallback,
}

/// What the pipeline actually did this call.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Applied {
    /// Full estimated request size before capping, pruning, or compaction,
    /// including the system prompt, tool schemas, and provider framing overhead.
    pub initial_token_count: u64,
    pub pruned: bool,
    pub pruned_tokens: u64,
    /// Results rewritten by the unconditional per-result cap pass.
    pub capped_parts: u64,
    /// Estimated tokens freed by the cap pass.
    pub capped_tokens: u64,
    pub compacted: bool,
    /// Present when compacted; the caller should persist it and pass
    /// it back as `options.previous_summary` (compaction round trip).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Present when compacted: index into the request messages where
    /// the verbatim tail begins (`null` = everything was summarised).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_start_index: Option<Option<usize>>,
    /// Present when compacted: estimated tokens of the summarised head.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_before: Option<u64>,
    /// Unambiguous alias for `tokens_before`; retained separately so callers
    /// do not mistake the summarised head for the full pre-compaction request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summarized_head_tokens: Option<u64>,
}

/// Where the returned `token_count` sits, by category — the same sums the
/// pipeline already maintains, exposed so callers can render a context
/// breakdown without re-counting: `token_count` equals the by_role sum plus
/// `system_prompt_tokens`, `tools_tokens`, and the request overhead.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AssembleBreakdown {
    /// Estimated tokens of the returned system prompt (any compaction
    /// summary section included).
    pub system_prompt_tokens: u64,
    /// Estimated tokens of the invocation schemas.
    pub tools_tokens: u64,
    /// The returned messages' tokens by role.
    pub by_role: ByRoleTokens,
    /// Per-part estimates for the request's `parts`, keyed by the
    /// caller's names; present when the request carried `parts`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_part: Option<BTreeMap<String, u64>>,
    pub estimator: EstimatorName,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AssemblySummary {
    pub assembly_id: String,
    pub span: RenderedSummary,
    /// Exact request indices actually handed to this compaction's summarizer.
    /// None means previous_summary; its authority belongs to the caller ledger.
    pub summarized_indices: Option<Vec<usize>>,
}

/// Which threshold made the assembly compact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTrigger {
    /// The request passed `options.compaction_budget` while still within
    /// `usable`.
    Budget,
    /// The request passed `usable`, the model's limit (as before 1.3.0).
    Usable,
}

/// How an inline compaction attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStatus {
    /// The head was summarised (`applied.compacted`).
    Compacted,
    /// The summariser failed; `failure` says why.
    Failed,
    /// Another caller holds the compaction lease.
    Busy,
    /// Nothing could be summarised without taking the whole request.
    Skipped,
}

/// The inline compaction this call tried.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CompactionReport {
    pub trigger: CompactionTrigger,
    pub status: CompactionStatus,
    /// Present when `status` is `failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AssembleResponse {
    /// `options.provenance` only: output ordinal -> exact request index;
    /// absent if the bounded map cannot fit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_indices: Option<Vec<usize>>,
    /// `options.provenance` only: where the rendered summary sits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_origin: Option<AssemblySummary>,

    pub system_prompt: String,
    /// Budgeted, ready to send to llm-router.
    pub messages: Vec<AgentMessage>,
    /// Estimated tokens of the returned context, including the system
    /// prompt, tools, and request overhead. Always at most `usable`.
    pub token_count: u64,
    /// The budget the context was fit into.
    pub usable: u64,
    /// The soft compaction threshold this call applied, `min(usable,
    /// options.compaction_budget)`; `null` without the option. Always
    /// present from 1.3.0, so a caller can tell an older context-manager
    /// (which drops the option) by the field's absence.
    pub compaction_budget: Option<u64>,
    /// Present only when this call tried an inline compaction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compaction: Option<CompactionReport>,
    /// Effective output allocation used to derive `usable`. This is the
    /// router-resolved request limit, not the model catalog ceiling.
    pub effective_max_output_tokens: u64,
    pub model_resolved: ModelResolvedWire,
    pub applied: Applied,
    pub breakdown: AssembleBreakdown,
}

/// Test-only re-export of [`count_context`] so sibling function tests
/// can pin cross-function counting equivalence (see count_tokens.rs).
#[cfg(test)]
pub(crate) fn count_context_for_tests(
    messages: &[AgentMessage],
    prompt: &str,
    tools: &[AgentFunction],
    request_overhead_tokens: u64,
    estimator: &dyn Estimator,
) -> u64 {
    count_context(messages, prompt, tools, request_overhead_tokens, estimator)
}

fn count_context(
    messages: &[AgentMessage],
    prompt: &str,
    tools: &[AgentFunction],
    request_overhead_tokens: u64,
    estimator: &dyn Estimator,
) -> u64 {
    let message_tokens = messages.iter().fold(0u64, |total, message| {
        total.saturating_add(estimator.message(message))
    });
    let tool_tokens = tools.iter().fold(0u64, |total, tool| {
        total.saturating_add(estimator.function(tool))
    });
    message_tokens
        .saturating_add(estimator.text(prompt))
        .saturating_add(tool_tokens)
        .saturating_add(request_overhead_tokens)
}

pub async fn handle(deps: &Deps, req: AssembleRequest) -> Result<AssembleResponse, ContextError> {
    let request_messages = req
        .messages
        .ok_or_else(|| ContextError::InvalidRequest("messages is required".into()))?;
    let options = req.options.unwrap_or_default();
    let config = deps.config().await;

    let resolved = resolve_model(deps, &req.model).await?;
    let reserved = options
        .reserved_tokens
        .unwrap_or_else(|| default_reserved(&config, resolved.limits.context_window));
    let thinking_budget = resolved.thinking_budget(options.thinking_level);
    let usable_budget = usable(&resolved.limits, reserved, thinking_budget);
    let compaction_budget = options
        .compaction_budget
        .map(|budget| budget.min(usable_budget));
    let thresholds = Budgets {
        hard: usable_budget,
        soft: compaction_budget.unwrap_or(usable_budget),
    };

    let estimator = estimator_for_model(&req.model.id);

    // Model-facing view: custom messages have no provider wire mapping
    // — they are excluded from the list and the count. `view_to_orig`
    // maps view indices back to request indices for tail_start_index.
    let mut working: Vec<AgentMessage> = Vec::new();
    let mut view_to_orig: Vec<usize> = Vec::new();
    for (idx, message) in request_messages.iter().enumerate() {
        if message.role() != Role::Custom {
            working.push(message.clone());
            view_to_orig.push(idx);
        }
    }
    normalize_media(&mut working, resolved.supports_vision);

    let base_prompt = req.system_prompt.as_deref();
    let provenance = options.provenance;
    let render = |summary: Option<&str>| {
        if provenance {
            render_system_prompt_with_span(base_prompt, summary)
        } else {
            (render_system_prompt(base_prompt, summary), None)
        }
    };
    let (mut system_prompt, mut summary_span) = render(options.previous_summary.as_deref());
    let mut summarized_indices = None;
    let tools = req.tools.as_deref().unwrap_or_default();
    let request_overhead_tokens = options.request_overhead_tokens.unwrap_or(0);
    let knobs = TurnKnobs {
        thinking_level: options.thinking_level,
        response_format: options.response_format.as_ref(),
    };

    // Size memo: every message, tool, and the prompt are estimated once;
    // the pipeline's recounts become O(1)-per-message sums, and the
    // mutating passes below keep `sizes` in lockstep with `working`.
    // The fold order and saturating ops mirror `count_context` exactly
    // so totals stay byte-identical with a from-scratch recount.
    let mut sizes: Vec<u64> = working.iter().map(|m| estimator.message(m)).collect();
    let tool_tokens = tools.iter().fold(0u64, |total, tool| {
        total.saturating_add(estimator.function(tool))
    });
    let mut prompt_tokens = estimator.text(&system_prompt);
    let total = |sizes: &[u64], prompt_tokens: u64| -> u64 {
        sizes
            .iter()
            .fold(0u64, |total, size| total.saturating_add(*size))
            .saturating_add(prompt_tokens)
            .saturating_add(tool_tokens)
            .saturating_add(request_overhead_tokens)
    };

    let initial_token_count = total(&sizes, prompt_tokens);
    let mut applied = Applied {
        initial_token_count,
        pruned: false,
        pruned_tokens: 0,
        capped_parts: 0,
        capped_tokens: 0,
        compacted: false,
        summary: None,
        tail_start_index: None,
        tokens_before: None,
        summarized_head_tokens: None,
    };

    let mut token_count = initial_token_count;

    // Step 0: cap oversized single results — always, any age (the spec's
    // unconditional ceiling; 0 disables). Runs before prune so (a) the
    // ceiling holds even when prune's own min_free_tokens hysteresis skips
    // its pass entirely, and (b) prune's window accounting then measures
    // what the model actually sees, not the pre-cap size.
    let max_result_tokens = options
        .max_result_tokens
        .unwrap_or(config.max_result_tokens);
    if max_result_tokens > 0 {
        let cap = cap_results_with_sizes(&mut working, &mut sizes, max_result_tokens, estimator);
        applied.capped_parts = cap.capped_parts;
        applied.capped_tokens = cap.capped_tokens;
        if cap.capped_parts > 0 {
            tracing::info!(
                capped_parts = cap.capped_parts,
                capped_tokens = cap.capped_tokens,
                "assemble: capped oversized function result(s)"
            );
        }
        token_count = total(&sizes, prompt_tokens);
    }

    // The estimate is chars/4, and dense content bills far more: random hex
    // about 2.25x, so a 128k window took a 256k prompt without compacting.
    // Past a third of the window (or 80 % of the compaction budget, which a
    // 1M window would otherwise decide on the estimate alone) the provider's
    // own count sets the budgets the steps below hold the estimate to.
    let gated_estimate = token_count;
    let gated = gate_budget(
        deps,
        &req.model,
        thresholds,
        provider_count_from(usable_budget, options.compaction_budget),
        token_count,
        request_overhead_tokens,
        &system_prompt,
        tools,
        &working,
        knobs,
    )
    .await;
    let soft_budget = gated.soft;
    let mut gate_budget = gated.hard;

    // Step 1: prune aged function outputs — only over budget
    // (context-manager.md § context::assemble). A prune rewrites results an
    // earlier request already sent, and provider prompt caches (DeepSeek,
    // Anthropic, OpenAI) reuse only the unchanged prefix: every pruned batch
    // made the provider re-read everything after it uncached. On the Linkly
    // tutorial that was 92% of the uncached input and 63% of the cost, with
    // the window never past a third full. Over budget it still runs first,
    // the cheapest relief before compaction.
    if options.allow_prune.unwrap_or(true) && token_count > soft_budget {
        let params = PruneParams {
            protect_recent_tokens: config.protect_recent_tokens,
            decay_user_turns: config.decay_user_turns,
            protected_user_turns: config.protected_user_turns,
            min_free_tokens: config.min_free_tokens,
            max_output_chars: config.max_output_chars,
            protected_functions: options.protected_functions.clone().unwrap_or_default(),
        };
        let stats = prune_with_sizes(&mut working, &mut sizes, &params, estimator);
        applied.pruned = stats.pruned_parts > 0;
        applied.pruned_tokens = stats.pruned_tokens;
        token_count = total(&sizes, prompt_tokens);
    }

    // Step 2: compact the head, past the soft threshold.
    let mut compaction = None;
    let mut compaction_failure = None;
    if !options.preview_only
        && token_count > soft_budget
        && options.allow_compaction.unwrap_or(true)
    {
        let trigger = if token_count > gate_budget {
            CompactionTrigger::Usable
        } else {
            CompactionTrigger::Budget
        };
        // The default lease key hashes the *request* message set —
        // the same derivation context::compact uses — so callers
        // hitting both functions with the same history contend on the
        // same claim.
        let lease_key = options
            .lease_key
            .clone()
            .unwrap_or_else(|| lease::default_lease_key(&request_messages));
        let attempt = try_compact(
            deps,
            &req.model,
            &working,
            &sizes,
            InlineCompaction {
                lease_key: &lease_key,
                tail_turns: options.tail_turns.unwrap_or(config.tail_turns),
                tail_budget: soft_budget,
                input_budget: gate_budget,
                max_result_tokens,
                steering: PromptSteering {
                    previous_summary: options.previous_summary.as_deref(),
                    instructions: options.compaction_instructions.as_deref(),
                },
                provider_options: options.provider_options.as_ref(),
            },
        )
        .await;
        let (status, failure) = match attempt {
            CompactAttempt::Compacted(compaction) => {
                (system_prompt, summary_span) = render(Some(&compaction.summary));
                working = working.split_off(compaction.head_len);
                sizes = sizes.split_off(compaction.head_len);
                prompt_tokens = estimator.text(&system_prompt);
                applied.compacted = true;
                applied.tail_start_index =
                    Some(compaction.tail_start_view.map(|v| view_to_orig[v]));
                if provenance {
                    let head: Vec<usize> = view_to_orig.drain(..compaction.head_len).collect();
                    summarized_indices = (head.len() <= 2048).then_some(head);
                }
                applied.tokens_before = Some(compaction.tokens_before);
                applied.summarized_head_tokens = Some(compaction.tokens_before);
                applied.summary = Some(compaction.summary);
                token_count = total(&sizes, prompt_tokens);
                (CompactionStatus::Compacted, None)
            }
            CompactAttempt::Busy => (CompactionStatus::Busy, None),
            CompactAttempt::Skipped => (CompactionStatus::Skipped, None),
            CompactAttempt::Failed(failure) => {
                compaction_failure = Some(failure.clone());
                (CompactionStatus::Failed, Some(failure))
            }
        };
        compaction = Some(CompactionReport {
            trigger,
            status,
            failure,
        });
    }

    // Step 3: enforce the budget by reducing complete function-result
    // messages, including latest/protected results and their details.
    // This pass is intentionally independent of all normal-prune knobs.
    if !options.preview_only && token_count > gate_budget {
        let excess = token_count.saturating_sub(gate_budget);
        reduce_results(&mut working, &mut sizes, excess, estimator, &mut applied);
        token_count = total(&sizes, prompt_tokens);
    }

    // Step 4: the provider's count scaled the whole request's estimate, but
    // relief keeps what it did not remove, and the emergency pass removes the
    // largest estimates first: the least dense content, about one billed
    // token per estimate unit. Count what is left; while it is over the
    // window, take the excess off in estimate units. Once it fits, the
    // estimate decides against `usable` again: content the emergency pass
    // cannot reduce (long prose prompts) may never reach the tight budget.
    if !options.preview_only && gate_budget < usable_budget && token_count < gated_estimate {
        for round in 0..=MAX_RECOUNTS {
            let Ok(counted) = deps
                .resolver
                .count_prompt_tokens(
                    req.model.provider.as_deref(),
                    &req.model.id,
                    &system_prompt,
                    tools,
                    &working,
                    knobs,
                )
                .await
            else {
                break;
            };
            let counted = counted.saturating_add(request_overhead_tokens);
            if counted <= usable_budget {
                gate_budget = usable_budget;
                break;
            }
            gate_budget = token_count.saturating_sub(counted - usable_budget);
            tracing::info!(
                counted,
                usable = usable_budget,
                budget = gate_budget,
                "assemble: the reduced request still counts over the window; reducing further"
            );
            // Out of rounds, or (below) nothing left to reduce: the check
            // after the loop answers.
            if round == MAX_RECOUNTS {
                break;
            }
            let excess = token_count.saturating_sub(gate_budget);
            let before = token_count;
            reduce_results(&mut working, &mut sizes, excess, estimator, &mut applied);
            token_count = total(&sizes, prompt_tokens);
            if token_count == before {
                break;
            }
        }
    }

    debug_assert_eq!(
        token_count,
        count_context(
            &working,
            &system_prompt,
            tools,
            request_overhead_tokens,
            estimator
        ),
        "size memo drifted from a from-scratch recount"
    );

    if !options.preview_only && token_count > gate_budget {
        return Err(ContextError::Overflow {
            token_count,
            usable: gate_budget,
            failure: compaction_failure,
        });
    }

    let by_role = by_role_from_sizes(&working, &sizes);
    let by_part = req.parts.as_ref().map(|parts| {
        parts
            .iter()
            .map(|(name, text)| (name.clone(), estimator.text(text)))
            .collect()
    });

    Ok(AssembleResponse {
        input_indices: (provenance && view_to_orig.len() <= 2048).then_some(view_to_orig),
        summary_origin: summary_span
            .filter(|_| system_prompt.len() <= 1024 * 1024)
            .map(|span| AssemblySummary {
                assembly_id: uuid::Uuid::new_v4().to_string(),
                span,
                summarized_indices,
            }),
        system_prompt,
        messages: working,
        token_count,
        usable: usable_budget,
        compaction_budget,
        compaction,
        effective_max_output_tokens: resolved.limits.max_output_tokens,
        model_resolved: match resolved.resolved {
            crate::core::budget::ModelResolved::Inline => ModelResolvedWire::Inline,
            crate::core::budget::ModelResolved::Router => ModelResolvedWire::Router,
            crate::core::budget::ModelResolved::Fallback => ModelResolvedWire::Fallback,
        },
        applied,
        breakdown: AssembleBreakdown {
            system_prompt_tokens: prompt_tokens,
            tools_tokens: tool_tokens,
            by_role: by_role.into(),
            by_part,
            estimator: estimator.kind().into(),
        },
    })
}

/// The provider is asked once the estimate passes this share (1/N) of the
/// window; content billing more than N times its estimate still slips past.
pub(super) const PROVIDER_COUNT_SHARE: u64 = 3;

/// With a compaction budget, the provider is also asked once the estimate
/// passes this share (in tenths) of the budget (D160).
const BUDGET_COUNT_TENTHS: u64 = 8;

/// The estimate from which the provider's count sets the budgets: a third
/// of `usable`, or 80 % of the compaction budget when that is lower. On a
/// 1M window (usable about 916k) a third is about 305k, so a 160k budget
/// would otherwise be decided on the estimate alone.
pub(super) fn provider_count_from(usable: u64, compaction_budget: Option<u64>) -> u64 {
    let share = usable.div_ceil(PROVIDER_COUNT_SHARE);
    compaction_budget.map_or(share, |budget| {
        share.min(budget.saturating_mul(BUDGET_COUNT_TENTHS).div_ceil(10))
    })
}

/// Reductions after relief left a request the provider counts over the
/// window; past them the request is an overflow.
const MAX_RECOUNTS: usize = 3;

/// Step 3's emergency pass: reduce complete function results by `excess`.
fn reduce_results(
    working: &mut [AgentMessage],
    sizes: &mut [u64],
    excess: u64,
    estimator: &dyn Estimator,
    applied: &mut Applied,
) {
    let emergency = emergency_reduce_with_sizes(working, sizes, excess, estimator);
    applied.pruned |= emergency.pruned_parts > 0;
    applied.pruned_tokens = applied
        .pruned_tokens
        .saturating_add(emergency.pruned_tokens);
}

/// The two thresholds the pipeline holds the estimate to, in estimate units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Budgets {
    /// `usable`: emergency reduction and the overflow error.
    hard: u64,
    /// `min(usable, compaction_budget)`: prune and compaction.
    soft: u64,
}

impl Budgets {
    /// Both thresholds at the rate the provider counts this request.
    fn scaled(self, estimate: u64, counted: u64) -> Self {
        let scale =
            |budget: u64| (u128::from(budget) * u128::from(estimate) / u128::from(counted)) as u64;
        Self {
            hard: scale(self.hard),
            soft: scale(self.soft),
        }
    }
}

/// The budgets, in estimate units, that keep the provider's count of the
/// request within each threshold: the thresholds themselves while the
/// estimate is below `count_from`, when nothing can count the request, or
/// when the count is no larger.
#[allow(clippy::too_many_arguments)]
async fn gate_budget(
    deps: &Deps,
    model: &ModelInput,
    thresholds: Budgets,
    count_from: u64,
    estimate: u64,
    request_overhead_tokens: u64,
    system_prompt: &str,
    tools: &[AgentFunction],
    messages: &[AgentMessage],
    knobs: TurnKnobs<'_>,
) -> Budgets {
    if messages.is_empty() || estimate < count_from {
        return thresholds;
    }
    // The estimate carries the request overhead; the provider's count of
    // the prompt, messages and tools does not.
    match deps
        .resolver
        .count_prompt_tokens(
            model.provider.as_deref(),
            &model.id,
            system_prompt,
            tools,
            messages,
            knobs,
        )
        .await
        .map(|counted| counted.saturating_add(request_overhead_tokens))
    {
        Ok(counted) if counted > estimate => {
            let budgets = thresholds.scaled(estimate, counted);
            tracing::info!(
                estimate,
                counted,
                usable = thresholds.hard,
                budget = budgets.hard,
                compaction_threshold = thresholds.soft,
                compaction_budget = budgets.soft,
                "assemble: the provider counts more than the estimate; budget tightened"
            );
            budgets
        }
        Ok(_) => thresholds,
        Err(error) => {
            tracing::debug!(%error, "assemble: no provider count; the estimate holds");
            thresholds
        }
    }
}

struct CompactionOutcome {
    summary: String,
    head_len: usize,
    tail_start_view: Option<usize>,
    tokens_before: u64,
}

/// How [`try_compact`] ended.
enum CompactAttempt {
    Compacted(CompactionOutcome),
    Busy,
    Skipped,
    Failed(Failure),
}

/// What one inline compaction runs with.
struct InlineCompaction<'a> {
    lease_key: &'a str,
    tail_turns: usize,
    /// Sizes the verbatim tail: the soft threshold.
    tail_budget: u64,
    /// Sizes the summariser's requests: the model's `usable`.
    input_budget: u64,
    max_result_tokens: u64,
    steering: PromptSteering<'a>,
    provider_options: Option<&'a ProviderOptions>,
}

/// Inline compaction under the lease. Anything short of `Compacted` —
/// lease busy, nothing to summarise, or summariser failure — leaves the
/// request as it was, and assemble continues to emergency reduction and
/// budget checking.
async fn try_compact(
    deps: &Deps,
    model: &ModelInput,
    working: &[AgentMessage],
    sizes: &[u64],
    plan: InlineCompaction<'_>,
) -> CompactAttempt {
    let config = deps.config().await;
    let leases = deps.leases().await;
    let ttl_ms = (config.lease_ttl_secs * 1_000) as i64;
    let Some(nonce) =
        lease::acquire(leases.as_ref(), deps.clock.as_ref(), plan.lease_key, ttl_ms).await
    else {
        return CompactAttempt::Busy;
    };

    let outcome = async {
        let budget = preserve_recent_budget(plan.tail_budget, None);
        let selection = select(working, sizes, budget, plan.tail_turns);
        // Never compact the ENTIRE working set during assembly: an empty
        // verbatim tail assembles into an empty messages array, which
        // providers hard-reject ("messages: at least one message is
        // required") and the harness turns into a terminal context_overflow.
        // select() legally returns a whole-head selection for tail_turns == 0
        // or a view with no user turn (e.g. a harness candidate window opened
        // at a prior compaction's assistant-boundary tail_start); skip
        // compaction and let emergency reduction do the shrinking.
        if selection.head_len >= working.len() {
            return CompactAttempt::Skipped;
        }
        let head = &working[..selection.head_len];
        if head.is_empty() {
            return CompactAttempt::Skipped;
        }

        let tokens_before: u64 = sizes[..selection.head_len].iter().sum();
        match summarize_head(
            deps,
            model,
            head,
            plan.input_budget,
            plan.max_result_tokens,
            plan.steering,
            plan.provider_options,
        )
        .await
        {
            Ok(summary) => CompactAttempt::Compacted(CompactionOutcome {
                summary,
                head_len: selection.head_len,
                tail_start_view: selection.tail_start_index,
                tokens_before,
            }),
            Err(err) => {
                // A summariser error skips compaction, after which the
                // emergency pass either fits the request or returns a
                // structured context overflow.
                tracing::warn!(error = %err, "assemble: compaction skipped");
                CompactAttempt::Failed(err.failure())
            }
        }
    }
    .await;

    lease::release(leases.as_ref(), plan.lease_key, &nonce).await;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::estimate::HeuristicEstimator;
    use serde_json::json;

    fn message(value: serde_json::Value) -> AgentMessage {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn tool_images_age_after_a_later_assistant_without_losing_neighbors() {
        let mut messages = vec![
            message(json!({
                "role": "function_result", "function_call_id": "c1",
                "function_id": "browser::screenshot",
                "content": [
                    { "type": "text", "text": "caption" },
                    { "type": "image", "mime": "image/png", "data": "AAAA" }
                ],
                "details": {}, "is_error": false, "timestamp": 1
            })),
            message(json!({
                "role": "user", "content": [{
                    "type": "function_result", "function_call_id": "c2", "content": [{
                        "type": "image", "mime": "image/png", "data": "BBBB"
                    }]
                }], "timestamp": 2
            })),
            message(json!({
                "role": "assistant", "content": [{ "type": "text", "text": "seen" }],
                "stop_reason": "end", "model": "m", "provider": "p", "timestamp": 3
            })),
        ];
        let original = messages.clone();

        let mut fresh = vec![messages[0].clone()];
        normalize_media(&mut fresh, Some(true));
        assert!(matches!(fresh[0].content()[1], ContentBlock::Image { .. }));

        normalize_media(&mut messages, Some(true));
        let value = serde_json::to_value(&messages).unwrap();
        assert_eq!(value[0]["content"][0]["text"], "caption");
        assert_eq!(value[0]["content"][1]["text"], TOOL_IMAGE_OMITTED);
        assert_eq!(
            value[1]["content"][0]["content"][0]["text"],
            TOOL_IMAGE_OMITTED
        );
        assert!(matches!(
            original[0].content()[1],
            ContentBlock::Image { .. }
        ));
    }

    #[test]
    fn user_images_survive_steering_and_age_on_the_next_turn() {
        let image = || {
            message(json!({
                "role": "user",
                "content": [{ "type": "image", "mime": "image/png", "data": "AAAA" }],
                "timestamp": 1
            }))
        };
        let user = || {
            message(json!({
                "role": "user", "content": [{ "type": "text", "text": "more" }], "timestamp": 4
            }))
        };
        let assistant = || {
            message(json!({
                "role": "assistant", "content": [{ "type": "text", "text": "answer" }],
                "stop_reason": "end", "model": "m", "provider": "p", "timestamp": 3
            }))
        };

        let mut steering = vec![image(), user()];
        normalize_media(&mut steering, Some(true));
        assert!(matches!(
            steering[0].content()[0],
            ContentBlock::Image { .. }
        ));

        let mut answered = vec![image(), assistant()];
        normalize_media(&mut answered, Some(true));
        assert!(matches!(
            answered[0].content()[0],
            ContentBlock::Image { .. }
        ));

        let mut advanced = vec![image(), assistant(), user()];
        normalize_media(&mut advanced, Some(true));
        assert_eq!(
            advanced[0].content(),
            &[ContentBlock::Text {
                text: USER_IMAGE_OMITTED.into()
            }]
        );
    }

    #[test]
    fn user_images_survive_steering_before_a_tool_continuation() {
        let mut messages = vec![
            message(json!({
                "role": "user",
                "content": [{ "type": "image", "mime": "image/png", "data": "AAAA" }],
                "timestamp": 1
            })),
            message(json!({
                "role": "user", "content": [{ "type": "text", "text": "steer" }],
                "timestamp": 2
            })),
            message(json!({
                "role": "assistant", "content": [{
                    "type": "function_call", "id": "c1",
                    "function_id": "browser::screenshot", "arguments": {}
                }],
                "stop_reason": "function_call", "model": "m", "provider": "p", "timestamp": 3
            })),
            message(json!({
                "role": "function_result", "function_call_id": "c1",
                "function_id": "browser::screenshot",
                "content": [{ "type": "text", "text": "result" }],
                "timestamp": 4
            })),
        ];

        normalize_media(&mut messages, Some(true));

        assert!(matches!(
            messages[0].content()[0],
            ContentBlock::Image { .. }
        ));
    }

    #[test]
    fn user_images_survive_inline_tool_results_before_the_continuation() {
        let mut messages: Vec<AgentMessage> = serde_json::from_value(json!([
            {
                "role": "user",
                "content": [{ "type": "image", "mime": "image/png", "data": "AAAA" }],
                "timestamp": 1
            },
            {
                "role": "assistant",
                "content": [{
                    "type": "function_call", "id": "c1",
                    "function_id": "browser::screenshot", "arguments": {}
                }],
                "stop_reason": "function_call", "model": "m", "provider": "p",
                "timestamp": 2
            },
            {
                "role": "user",
                "content": [{
                    "type": "function_result", "function_call_id": "c1",
                    "content": [{ "type": "text", "text": "result" }]
                }],
                "timestamp": 3
            },
            {
                "role": "assistant", "content": [{ "type": "text", "text": "answer" }],
                "stop_reason": "end", "model": "m", "provider": "p", "timestamp": 4
            }
        ]))
        .unwrap();

        normalize_media(&mut messages, Some(true));

        assert!(matches!(
            messages[0].content()[0],
            ContentBlock::Image { .. }
        ));
    }

    #[test]
    fn missing_capability_keeps_images_and_text_only_replaces_them() {
        let original = vec![message(json!({
            "role": "assistant",
            "content": [
                { "type": "image", "mime": "image/png", "data": "AAAA" },
                { "type": "function_result", "function_call_id": "c1", "content": [{
                    "type": "image", "mime": "image/png", "data": "BBBB"
                }] }
            ],
            "stop_reason": "end", "model": "m", "provider": "p", "timestamp": 1
        }))];

        let mut unknown = original.clone();
        normalize_media(&mut unknown, None);
        assert_eq!(unknown, original);

        let mut vision = original.clone();
        normalize_media(&mut vision, Some(true));
        assert_eq!(vision, original);

        let mut text_only = original;
        normalize_media(&mut text_only, Some(false));
        assert_eq!(
            serde_json::to_value(&text_only).unwrap()[0]["content"][0]["text"],
            TEXT_ONLY_IMAGE_OMITTED
        );
        assert_eq!(
            serde_json::to_value(&text_only).unwrap()[0]["content"][1]["content"][0]["text"],
            TEXT_ONLY_IMAGE_OMITTED
        );
    }

    #[test]
    fn the_provider_count_starts_at_a_third_of_usable_or_80_percent_of_the_budget() {
        // No budget: a third of usable, as `estimate * 3 >= usable` read before.
        assert_eq!(provider_count_from(900_000, None), 300_000);
        assert_eq!(provider_count_from(100, None), 34);
        for estimate in [32u64, 33, 34, 35] {
            assert_eq!(
                estimate >= provider_count_from(100, None),
                estimate * PROVIDER_COUNT_SHARE >= 100,
                "estimate {estimate}"
            );
        }
        // A 160k budget on a 1M model: 128k, not ~305k.
        assert_eq!(provider_count_from(916_000, Some(160_000)), 128_000);
        // A budget above the window changes nothing.
        assert_eq!(provider_count_from(116_000, Some(2_000_000)), 38_667);
        assert_eq!(provider_count_from(0, Some(160_000)), 0);
    }

    #[test]
    fn budgets_scale_at_the_rate_the_provider_counts() {
        let budgets = Budgets {
            hard: 900_000,
            soft: 100_000,
        };
        assert_eq!(
            budgets.scaled(85_000, 127_500),
            Budgets {
                hard: 600_000,
                soft: 66_666,
            }
        );
    }

    #[test]
    fn the_new_options_are_optional_on_the_wire() {
        let options: AssembleOptions = serde_json::from_value(json!({})).unwrap();
        assert_eq!(options.compaction_budget, None);
        assert_eq!(options.compaction_instructions, None);
        assert_eq!(options.provider_options, None);

        let options: AssembleOptions = serde_json::from_value(json!({
            "compaction_budget": 176_000,
            "compaction_instructions": "keep the deploy steps",
            "provider_options": { "claude-code": { "account": "acct-work" } }
        }))
        .unwrap();
        assert_eq!(options.compaction_budget, Some(176_000));
        assert_eq!(
            options.provider_options.unwrap()["claude-code"]["account"],
            "acct-work"
        );
    }

    #[test]
    fn count_includes_tools_and_request_overhead() {
        let message: AgentMessage = serde_json::from_value(json!({
            "role": "user",
            "content": [{ "type": "text", "text": "hello" }],
            "timestamp": 1
        }))
        .unwrap();
        let tool: AgentFunction = serde_json::from_value(json!({
            "name": "agent_call",
            "description": "Run an agent function",
            "parameters": {
                "type": "object",
                "properties": { "function_id": { "type": "string" } }
            }
        }))
        .unwrap();
        let estimator = HeuristicEstimator;
        let baseline = count_context(std::slice::from_ref(&message), "system", &[], 0, &estimator);

        let counted = count_context(
            &[message],
            "system",
            std::slice::from_ref(&tool),
            37,
            &estimator,
        );

        assert_eq!(counted, baseline + estimator.function(&tool) + 37);
    }

    #[test]
    fn assemble_wire_accepts_optional_tools_and_request_overhead() {
        let request: AssembleRequest = serde_json::from_value(json!({
            "messages": [],
            "model": {
                "id": "m",
                "limits": { "context_window": 1_000, "max_output_tokens": 100 }
            },
            "tools": [{
                "name": "agent_call",
                "description": "Run an agent function",
                "parameters": { "type": "object" }
            }],
            "options": { "request_overhead_tokens": 41 }
        }))
        .unwrap();

        assert_eq!(request.tools.as_ref().unwrap().len(), 1);
        assert_eq!(
            request
                .options
                .as_ref()
                .and_then(|options| options.request_overhead_tokens),
            Some(41)
        );
    }
}
