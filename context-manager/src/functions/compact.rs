//! `context::compact` — summarise the head of a history into a single
//! compaction summary, keeping a recent tail verbatim
//! (context-manager.md § context::compact).
//!
//! Transient and storage-agnostic: the summary is returned for the
//! caller to persist; no session is touched. A short-lived lease
//! (scope `context_lease`) keeps two callers from summarising the same
//! logical history concurrently. Without `llm-router` the summariser
//! is unavailable and the response is `{ status: "overflow" }` —
//! callers treat it as "compaction unavailable". Every `overflow` carries
//! the summariser's `failure` (its kind, a message, the router's
//! `error_details`), so a caller can tell a usage limit from an input
//! too large for the window.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::summarize::{PromptSteering, summarize_head};
use crate::core::budget::{default_reserved, preserve_recent_budget, usable};
use crate::core::estimate::{Estimator, estimator_for_model};
use crate::core::lease;
use crate::core::selection::{fit_tail, select};
use crate::error::ContextError;
use crate::functions::resolve_model;
use crate::ports::{Deps, SummarizeError};
use crate::types::{AgentMessage, Failure, ModelInput, ProviderOptions};

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
    /// `router::chat` `provider_options`, namespaced by provider id (e.g.
    /// the conversation's account under `claude-code`), forwarded verbatim
    /// to every summariser call. Absent: the router's defaults.
    #[serde(default)]
    pub provider_options: Option<ProviderOptions>,
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
    /// The summariser is unavailable, overflowed, or returned no summary.
    Overflow {
        /// Why: present on every `overflow` from 1.3.0; older versions sent
        /// none.
        #[serde(skip_serializing_if = "Option::is_none")]
        failure: Option<Failure>,
    },
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
        options.provider_options.as_ref(),
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
    provider_options: Option<&ProviderOptions>,
    estimator: &dyn Estimator,
) -> CompactResponse {
    // One estimate per message; select, tokens_before, and tokens_after
    // all read this memo instead of re-serializing per pass.
    let sizes: Vec<u64> = messages.iter().map(|m| estimator.message(m)).collect();
    // An explicit compaction must free room: when the kept turn alone is
    // oversized (the one that overflowed the window), cut inside it until
    // the verbatim tail leaves at least half the input budget free.
    let selection = fit_tail(
        messages,
        &sizes,
        select(messages, &sizes, budget, tail_turns),
        input_budget / 2,
    );
    let head = &messages[..selection.head_len];
    if head.is_empty() {
        return CompactResponse::Empty;
    }

    let tokens_before: u64 = sizes[..selection.head_len].iter().sum();
    // A direct compaction takes no per-request cap: the configured one.
    let max_result_tokens = deps.config().await.max_result_tokens;
    let summary = match summarize_head(
        deps,
        model,
        head,
        input_budget,
        max_result_tokens,
        steering,
        provider_options,
    )
    .await
    {
        Ok(summary) => summary,
        Err(err) => {
            match &err {
                // `empty` means "nothing to compact"; a blank summary is the
                // summariser failing on a non-empty head.
                SummarizeError::Empty => tracing::error!("summariser produced an empty summary"),
                // Spec: without llm-router, compact returns overflow with a
                // permanent error_kind in the worker log.
                SummarizeError::Unavailable(_) => {
                    tracing::error!(error = %err, error_kind = "permanent", "compaction unavailable")
                }
                _ => tracing::error!(error = %err, "summariser failed"),
            }
            return CompactResponse::Overflow {
                failure: Some(err.failure()),
            };
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
