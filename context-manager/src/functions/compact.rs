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

use crate::core::budget::{default_reserved, preserve_recent_budget, usable};
use crate::core::estimate::{estimator_for_model, Estimator};
use crate::core::lease;
use crate::core::selection::select;
use crate::core::summary::{build_system_prompt, render_user_prompt, strip_media};
use crate::error::ContextError;
use crate::functions::resolve_model;
use crate::ports::{Deps, SummarizeError, SummarizeRequest};
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
    let budget = preserve_recent_budget(
        usable(&resolved.limits, reserved, 0),
        options.preserve_recent_tokens,
    );
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
        tail_turns,
        options.previous_summary.as_deref(),
        estimator,
    )
    .await;

    lease::release(leases.as_ref(), &lease_key, &nonce).await;
    Ok(outcome)
}

/// The summarisation pipeline between lease acquire and release.
async fn summarise(
    deps: &Deps,
    model: &ModelInput,
    messages: &[AgentMessage],
    budget: u64,
    tail_turns: usize,
    previous_summary: Option<&str>,
    estimator: &dyn Estimator,
) -> CompactResponse {
    let selection = select(messages, budget, tail_turns, estimator);
    let head = &messages[..selection.head_len];
    if head.is_empty() {
        return CompactResponse::Empty;
    }
    let tail = &messages[selection.head_len..];

    let tokens_before: u64 = head.iter().map(|m| estimator.message(m)).sum();
    let stripped = strip_media(head, deps.config().await.max_output_chars);

    let request = SummarizeRequest {
        system_prompt: build_system_prompt(previous_summary),
        user_prompt: render_user_prompt(&stripped),
        model: model.id.clone(),
        provider: model.provider.clone(),
    };

    let summary = match deps.summarizer.summarize(request).await {
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

    let tokens_after =
        estimator.text(&summary) + tail.iter().map(|m| estimator.message(m)).sum::<u64>();

    CompactResponse::Ok {
        summary,
        tail_start_index: selection.tail_start_index,
        tokens_before,
        tokens_after,
        used_prior_summary: previous_summary.is_some(),
    }
}
