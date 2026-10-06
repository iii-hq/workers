//! `context::compact` — summarise the head of a history into a single
//! compaction summary, keeping a recent tail verbatim
//! (context-manager.md § context::compact).
//!
//! Transient and storage-agnostic: the summary is returned for the
//! caller to persist; no session is touched. A short-lived lease
//! (scope `context_lease`) keeps two callers from summarising the same
//! logical history concurrently. Without `llm-router` the summariser
//! is unavailable and the response is `{ status: "overflow" }` —
//! callers treat it as "compaction unavailable".

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::summarize::{summarize_head, PromptSteering};
use crate::core::budget::{default_reserved, preserve_recent_budget, usable};
use crate::core::estimate::{estimator_for_model, Estimator};
use crate::core::lease;
use crate::core::selection::select;
use crate::error::ContextError;
use crate::functions::resolve_model;
use crate::ports::{Deps, SummarizeError};
use crate::types::{AgentMessage, ModelInput};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct CompactOptions {
    /// user+assistant pairs kept verbatim (default 2).
    #[serde(default)]
    pub tail_turns: Option<usize>,
    /// Anchor from a prior compaction so summaries converge instead of
    /// growing; the summariser updates it rather than starting over.
    #[serde(default)]
    pub previous_summary: Option<String>,
    /// Override the adaptive verbatim-tail token budget.
    #[serde(default)]
    pub preserve_recent_tokens: Option<u64>,
    /// Mutual-exclusion key (e.g. a session id); default: hash of the
    /// message set.
    #[serde(default)]
    pub lease_key: Option<String>,
    /// One-shot caller guidance for the summariser — what to keep, drop,
    /// or emphasise (e.g. the text after a user's `/compact`). Applied
    /// within the fixed summary template, never replacing it. Trimmed;
    /// blank is ignored; cut past 2000 chars. Not carried forward — pass
    /// it again to steer a later compaction.
    #[serde(default)]
    pub instructions: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompactRequest {
    /// Full candidate history, oldest first.
    pub messages: Option<Vec<AgentMessage>>,
    pub model: ModelInput,
    #[serde(default)]
    pub options: Option<CompactOptions>,
}

/// Discriminated on `status`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CompactResponse {
    /// Compaction ran; the caller should persist `summary` and map
    /// `tail_start_index` onto its own storage ids.
    Ok {
        summary: String,
        /// Index into the request `messages` where the verbatim tail
        /// begins; `null` when everything was summarised.
        tail_start_index: Option<usize>,
        /// Estimated tokens of the summarised head.
        tokens_before: u64,
        /// Estimated tokens of summary + verbatim tail.
        tokens_after: u64,
        used_prior_summary: bool,
    },
    /// A compaction lease is held; the caller may retry.
    Busy,
    /// Nothing to compact.
    Empty,
    /// The summariser is unavailable or itself overflowed.
    Overflow,
}

pub async fn handle(deps: &Deps, req: CompactRequest) -> Result<CompactResponse, ContextError> {
    let messages = req
        .messages
        .ok_or_else(|| ContextError::InvalidRequest("messages is required".into()))?;
    if messages.is_empty() {
        return Ok(CompactResponse::Empty);
    }

    let config = deps.config().await;
    let options = req.options.unwrap_or_default();
    let resolved = resolve_model(deps, &req.model).await?;
    let estimator = estimator_for_model(&req.model.id);

    let reserved = default_reserved(&config, resolved.limits.context_window);
    let input_budget = usable(&resolved.limits, reserved, 0);
    let budget = preserve_recent_budget(input_budget, options.preserve_recent_tokens);
    let tail_turns = options.tail_turns.unwrap_or(config.tail_turns);

    let lease_key = options
        .lease_key
        .clone()
        .unwrap_or_else(|| lease::default_lease_key(&messages));
    let ttl_ms = (config.lease_ttl_secs * 1_000) as i64;
    let leases = deps.leases().await;
    let Some(nonce) =
        lease::acquire(leases.as_ref(), deps.clock.as_ref(), &lease_key, ttl_ms).await
    else {
        return Ok(CompactResponse::Busy);
    };

    let outcome = summarise(
        deps,
        &req.model,
        &messages,
        budget,
        input_budget,
        tail_turns,
        PromptSteering {
            previous_summary: options.previous_summary.as_deref(),
            instructions: options.instructions.as_deref(),
        },
        estimator,
    )
    .await;

    lease::release(leases.as_ref(), &lease_key, &nonce).await;
    Ok(outcome)
}

/// The summarisation pipeline between lease acquire and release.
#[allow(clippy::too_many_arguments)]
async fn summarise(
    deps: &Deps,
    model: &ModelInput,
    messages: &[AgentMessage],
    budget: u64,
    input_budget: u64,
    tail_turns: usize,
    steering: PromptSteering<'_>,
    estimator: &dyn Estimator,
) -> CompactResponse {
    // One estimate per message; select, tokens_before, and tokens_after
    // all read this memo instead of re-serializing per pass.
    let sizes: Vec<u64> = messages.iter().map(|m| estimator.message(m)).collect();
    let selection = select(messages, &sizes, budget, tail_turns);
    let head = &messages[..selection.head_len];
    if head.is_empty() {
        return CompactResponse::Empty;
    }

    let tokens_before: u64 = sizes[..selection.head_len].iter().sum();
    let summary = match summarize_head(deps, model, head, input_budget, steering).await {
        Ok(summary) => summary,
        Err(SummarizeError::Empty) => {
            tracing::warn!("summariser produced an empty summary; nothing to compact");
            return CompactResponse::Empty;
        }
        Err(err @ SummarizeError::Unavailable(_)) => {
            // Spec: without llm-router, compact returns overflow with a
            // permanent error_kind in the worker log.
            tracing::error!(error = %err, error_kind = "permanent", "compaction unavailable");
            return CompactResponse::Overflow;
        }
        Err(err) => {
            tracing::error!(error = %err, "summariser failed");
            return CompactResponse::Overflow;
        }
    };

    let tokens_after = estimator.text(&summary) + sizes[selection.head_len..].iter().sum::<u64>();

    CompactResponse::Ok {
        summary,
        tail_start_index: selection.tail_start_index,
        tokens_before,
        tokens_after,
        used_prior_summary: steering.previous_summary.is_some(),
    }
}
