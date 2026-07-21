//! `context::assemble` — build the model-ready context from a history
//! (context-manager.md § context::assemble). The pipeline, in order:
//! count -> (if over) prune function outputs -> (if still over) compact
//! the head -> (if still over) emergency-reduce function results ->
//! assemble the final list or return a structured overflow.
//!
//! Structural guarantees: `role: "custom"` messages never reach the
//! model-facing list (nor the count); `applied.tail_start_index`
//! indexes the *request* messages array so callers can map it onto
//! their own storage; a busy lease or failed summariser falls through
//! to emergency reduction and the hard budget check.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::budget::{default_reserved, preserve_recent_budget, usable};
use crate::core::estimate::{estimator_for_model, Estimator};
use crate::core::lease;
use crate::core::prune::{emergency_reduce_with_sizes, prune_with_sizes, PruneParams};
use crate::core::selection::select;
use crate::core::summary::{
    build_system_prompt, render_system_prompt, render_user_prompt, strip_media,
};
use crate::error::ContextError;
use crate::functions::resolve_model;
use crate::ports::{Deps, SummarizeRequest};
use crate::types::{AgentFunction, AgentMessage, ModelInput, Role, ThinkingLevel};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct AssembleOptions {
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
    /// Estimated tokens for final provider request fields and framing
    /// not otherwise represented by the prompt, messages, or tools.
    #[serde(default)]
    pub request_overhead_tokens: Option<u64>,
    /// Reserve the model's thinking budget for this tier.
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
    /// Compaction mutual-exclusion key (e.g. a session id); default:
    /// hash of the message set.
    #[serde(default)]
    pub lease_key: Option<String>,
    /// Persisted summary from a prior compaction (see the spec's "The
    /// compaction round trip"); rendered into the system prompt and
    /// used as the anchor if compaction triggers again.
    #[serde(default)]
    pub previous_summary: Option<String>,
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
    /// Full estimated request size before pruning or compaction, including the
    /// system prompt, tool schemas, and provider framing overhead.
    pub initial_token_count: u64,
    pub pruned: bool,
    pub pruned_tokens: u64,
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

#[derive(Debug, Serialize, JsonSchema)]
pub struct AssembleResponse {
    pub system_prompt: String,
    /// Budgeted, ready to send to llm-router.
    pub messages: Vec<AgentMessage>,
    /// Estimated tokens of the returned context, including the system
    /// prompt, tools, and request overhead. Always at most `usable`.
    pub token_count: u64,
    /// The budget the context was fit into.
    pub usable: u64,
    /// Effective output allocation used to derive `usable`. This is the
    /// router-resolved request limit, not the model catalog ceiling.
    pub effective_max_output_tokens: u64,
    pub model_resolved: ModelResolvedWire,
    pub applied: Applied,
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

    let base_prompt = req.system_prompt.as_deref();
    let mut system_prompt = render_system_prompt(base_prompt, options.previous_summary.as_deref());
    let tools = req.tools.as_deref().unwrap_or_default();
    let request_overhead_tokens = options.request_overhead_tokens.unwrap_or(0);

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
        compacted: false,
        summary: None,
        tail_start_index: None,
        tokens_before: None,
        summarized_head_tokens: None,
    };

    let mut token_count = initial_token_count;

    // Step 1: prune function outputs.
    if token_count > usable_budget && options.allow_prune.unwrap_or(true) {
        let params = PruneParams {
            protect_recent_tokens: config.protect_recent_tokens,
            min_free_tokens: config.min_free_tokens,
            max_output_chars: config.max_output_chars,
            protected_functions: options.protected_functions.clone().unwrap_or_default(),
        };
        let stats = prune_with_sizes(&mut working, &mut sizes, &params, estimator);
        applied.pruned = stats.pruned_parts > 0;
        applied.pruned_tokens = stats.pruned_tokens;
        token_count = total(&sizes, prompt_tokens);
    }

    // Step 2: compact the head.
    if token_count > usable_budget && options.allow_compaction.unwrap_or(true) {
        // The default lease key hashes the *request* message set —
        // the same derivation context::compact uses — so callers
        // hitting both functions with the same history contend on the
        // same claim.
        let lease_key = options
            .lease_key
            .clone()
            .unwrap_or_else(|| lease::default_lease_key(&request_messages));
        if let Some(compaction) = try_compact(
            deps,
            &req.model,
            &working,
            &sizes,
            usable_budget,
            options.tail_turns.unwrap_or(config.tail_turns),
            &lease_key,
            options.previous_summary.as_deref(),
        )
        .await
        {
            system_prompt = render_system_prompt(base_prompt, Some(&compaction.summary));
            working = working.split_off(compaction.head_len);
            sizes = sizes.split_off(compaction.head_len);
            prompt_tokens = estimator.text(&system_prompt);
            applied.compacted = true;
            applied.tail_start_index = Some(compaction.tail_start_view.map(|v| view_to_orig[v]));
            applied.tokens_before = Some(compaction.tokens_before);
            applied.summarized_head_tokens = Some(compaction.tokens_before);
            applied.summary = Some(compaction.summary);
            token_count = total(&sizes, prompt_tokens);
        }
    }

    // Step 3: enforce the budget by reducing complete function-result
    // messages, including latest/protected results and their details.
    // This pass is intentionally independent of all normal-prune knobs.
    if token_count > usable_budget {
        let emergency = emergency_reduce_with_sizes(
            &mut working,
            &mut sizes,
            token_count.saturating_sub(usable_budget),
            estimator,
        );
        applied.pruned |= emergency.pruned_parts > 0;
        applied.pruned_tokens = applied
            .pruned_tokens
            .saturating_add(emergency.pruned_tokens);
        token_count = total(&sizes, prompt_tokens);
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

    if token_count > usable_budget {
        return Err(ContextError::Overflow {
            token_count,
            usable: usable_budget,
        });
    }

    Ok(AssembleResponse {
        system_prompt,
        messages: working,
        token_count,
        usable: usable_budget,
        effective_max_output_tokens: resolved.limits.max_output_tokens,
        model_resolved: match resolved.resolved {
            crate::core::budget::ModelResolved::Inline => ModelResolvedWire::Inline,
            crate::core::budget::ModelResolved::Router => ModelResolvedWire::Router,
            crate::core::budget::ModelResolved::Fallback => ModelResolvedWire::Fallback,
        },
        applied,
    })
}

struct CompactionOutcome {
    summary: String,
    head_len: usize,
    tail_start_view: Option<usize>,
    tokens_before: u64,
}

/// Inline compaction under the lease. `None` means "no compaction this
/// call" — lease busy, nothing to summarise, or summariser failure —
/// and assemble continues to emergency reduction and budget checking.
#[allow(clippy::too_many_arguments)]
async fn try_compact(
    deps: &Deps,
    model: &ModelInput,
    working: &[AgentMessage],
    sizes: &[u64],
    usable_budget: u64,
    tail_turns: usize,
    lease_key: &str,
    previous_summary: Option<&str>,
) -> Option<CompactionOutcome> {
    let config = deps.config().await;
    let leases = deps.leases().await;
    let ttl_ms = (config.lease_ttl_secs * 1_000) as i64;
    let nonce = lease::acquire(leases.as_ref(), deps.clock.as_ref(), lease_key, ttl_ms).await?;

    let outcome = async {
        let budget = preserve_recent_budget(usable_budget, None);
        let selection = select(working, sizes, budget, tail_turns);
        // Never compact the ENTIRE working set during assembly: an empty
        // verbatim tail assembles into an empty messages array, which
        // providers hard-reject ("messages: at least one message is
        // required") and the harness turns into a terminal context_overflow.
        // select() legally returns a whole-head selection for tail_turns == 0
        // or a view with no user turn (e.g. a harness candidate window opened
        // at a prior compaction's assistant-boundary tail_start); skip
        // compaction and let emergency reduction do the shrinking.
        if selection.head_len >= working.len() {
            return None;
        }
        let head = &working[..selection.head_len];
        if head.is_empty() {
            return None;
        }

        let tokens_before: u64 = sizes[..selection.head_len].iter().sum();
        let stripped = strip_media(head, config.max_output_chars);
        let request = SummarizeRequest {
            system_prompt: build_system_prompt(previous_summary),
            user_prompt: render_user_prompt(&stripped),
            model: model.id.clone(),
            provider: model.provider.clone(),
        };

        match deps.summarizer.summarize(request).await {
            Ok(summary) => Some(CompactionOutcome {
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
                None
            }
        }
    }
    .await;

    lease::release(leases.as_ref(), lease_key, &nonce).await;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::estimate::HeuristicEstimator;
    use serde_json::json;

    #[test]
    fn count_includes_tools_and_request_overhead() {
        let message: AgentMessage = serde_json::from_value(json!({
            "role": "user",
            "content": [{ "type": "text", "text": "hello" }],
            "timestamp": 1
        }))
        .unwrap();
        let tool: AgentFunction = serde_json::from_value(json!({
            "name": "agent_trigger",
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
                "name": "agent_trigger",
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
