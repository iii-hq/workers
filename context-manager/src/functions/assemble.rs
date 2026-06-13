//! `context::assemble` — build the model-ready context from a history
//! (context-manager.md § context::assemble). The pipeline, in order:
//! count -> (if over) prune function outputs -> (if still over) compact
//! the head -> assemble the final list.
//!
//! Structural guarantees: `role: "custom"` messages never reach the
//! model-facing list (nor the count); `applied.tail_start_index`
//! indexes the *request* messages array so callers can map it onto
//! their own storage; a busy lease or failed summariser degrades to
//! best effort instead of erroring the turn.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::budget::{default_reserved, preserve_recent_budget, usable};
use crate::core::estimate::{estimator_for_model, Estimator};
use crate::core::lease;
use crate::core::prune::{prune as run_prune, PruneParams};
use crate::core::selection::select;
use crate::core::summary::{
    build_system_prompt, render_system_prompt, render_user_prompt, strip_media,
};
use crate::error::ContextError;
use crate::functions::resolve_model;
use crate::ports::{Deps, SummarizeRequest};
use crate::types::{AgentMessage, ModelInput, Role, ThinkingLevel};

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
    /// `function_id`s whose outputs are never pruned.
    #[serde(default)]
    pub protected_functions: Option<Vec<String>>,
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
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AssembleResponse {
    pub system_prompt: String,
    /// Budgeted, ready to send to llm-router.
    pub messages: Vec<AgentMessage>,
    /// Estimated tokens of the returned context (messages + system
    /// prompt). Can exceed `usable` when prune/compaction were
    /// disabled, unavailable, or insufficient — best effort, visible.
    pub token_count: u64,
    /// The budget the context was fit into.
    pub usable: u64,
    pub model_resolved: ModelResolvedWire,
    pub applied: Applied,
}

pub async fn handle(deps: &Deps, req: AssembleRequest) -> Result<AssembleResponse, ContextError> {
    let request_messages = req
        .messages
        .ok_or_else(|| ContextError::InvalidRequest("messages is required".into()))?;
    let options = req.options.unwrap_or_default();

    let resolved = resolve_model(deps, &req.model).await?;
    let reserved = options
        .reserved_tokens
        .unwrap_or_else(|| default_reserved(&deps.config, resolved.limits.context_window));
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

    let count = |messages: &[AgentMessage], prompt: &str| -> u64 {
        messages.iter().map(|m| estimator.message(m)).sum::<u64>() + estimator.text(prompt)
    };

    let mut applied = Applied {
        pruned: false,
        pruned_tokens: 0,
        compacted: false,
        summary: None,
        tail_start_index: None,
        tokens_before: None,
    };

    let mut token_count = count(&working, &system_prompt);

    // Step 1: prune function outputs.
    if token_count > usable_budget && options.allow_prune.unwrap_or(true) {
        let params = PruneParams {
            protect_recent_tokens: deps.config.protect_recent_tokens,
            min_free_tokens: deps.config.min_free_tokens,
            max_output_chars: deps.config.max_output_chars,
            protected_functions: options.protected_functions.clone().unwrap_or_default(),
        };
        let stats = run_prune(&mut working, &params, estimator);
        applied.pruned = stats.pruned_parts > 0;
        applied.pruned_tokens = stats.pruned_tokens;
        token_count = count(&working, &system_prompt);
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
            usable_budget,
            options.tail_turns.unwrap_or(deps.config.tail_turns),
            &lease_key,
            options.previous_summary.as_deref(),
            estimator,
        )
        .await
        {
            system_prompt = render_system_prompt(base_prompt, Some(&compaction.summary));
            working = working.split_off(compaction.head_len);
            applied.compacted = true;
            applied.tail_start_index = Some(compaction.tail_start_view.map(|v| view_to_orig[v]));
            applied.tokens_before = Some(compaction.tokens_before);
            applied.summary = Some(compaction.summary);
            token_count = count(&working, &system_prompt);
        }
    }

    Ok(AssembleResponse {
        system_prompt,
        messages: working,
        token_count,
        usable: usable_budget,
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
/// and assemble degrades to best effort.
#[allow(clippy::too_many_arguments)]
async fn try_compact(
    deps: &Deps,
    model: &ModelInput,
    working: &[AgentMessage],
    usable_budget: u64,
    tail_turns: usize,
    lease_key: &str,
    previous_summary: Option<&str>,
    estimator: &dyn Estimator,
) -> Option<CompactionOutcome> {
    let ttl_ms = (deps.config.lease_ttl_secs * 1_000) as i64;
    let nonce =
        lease::acquire(deps.leases.as_ref(), deps.clock.as_ref(), lease_key, ttl_ms).await?;

    let outcome = async {
        let budget = preserve_recent_budget(usable_budget, None);
        let selection = select(working, budget, tail_turns, estimator);
        let head = &working[..selection.head_len];
        if head.is_empty() {
            return None;
        }

        let tokens_before: u64 = head.iter().map(|m| estimator.message(m)).sum();
        let stripped = strip_media(head, deps.config.max_output_chars);
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
                // Assemble never fails the turn on a summariser error;
                // the caller still gets a (possibly over-budget)
                // context and can see token_count > usable.
                tracing::warn!(error = %err, "assemble: compaction skipped");
                None
            }
        }
    }
    .await;

    lease::release(deps.leases.as_ref(), lease_key, &nonce).await;
    outcome
}
