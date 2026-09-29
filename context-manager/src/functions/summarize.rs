//! Bounded, anchored summarization shared by automatic and explicit compaction.
use std::time::Duration;

use crate::core::estimate::{estimator_for_model, Estimator};
use crate::core::summary::{build_system_prompt, render_user_prompt, strip_media};
use crate::ports::{Deps, SummarizeError, SummarizeRequest};
use crate::types::{AgentMessage, ContentBlock, ModelInput};

// Bound cost and runtime even when a summary fails to shrink between chunks.
const MAX_CHUNKS: usize = 32;
const REQUEST_OVERHEAD_TOKENS: u64 = 64;

#[derive(Clone, Copy, Default)]
pub(super) struct PromptSteering<'a> {
    pub previous_summary: Option<&'a str>,
    pub instructions: Option<&'a str>,
}

pub(super) async fn summarize_head(
    deps: &Deps,
    model: &ModelInput,
    head: &[AgentMessage],
    input_budget: u64,
    steering: PromptSteering<'_>,
) -> Result<String, SummarizeError> {
    let config = deps.config().await;
    let stripped = strip_media(head, config.max_output_chars);
    let prompt = render_user_prompt(&stripped);
    // One deadline for the entire pipeline, not a fresh full allowance per chunk.
    // Finish before the lease expires so another caller cannot overlap this work.
    let timeout_ms = config.summarizer_timeout_ms.min(
        config
            .lease_ttl_secs
            .saturating_mul(1_000)
            .saturating_sub(1_000),
    );
    let operation = summarize_prompt(deps, model, &prompt, input_budget, steering);
    tokio::time::timeout(Duration::from_millis(timeout_ms), operation)
        .await
        .map_err(|_| SummarizeError::Failed("summary pipeline exceeded its time budget".into()))?
}

async fn summarize_prompt(
    deps: &Deps,
    model: &ModelInput,
    prompt: &str,
    input_budget: u64,
    steering: PromptSteering<'_>,
) -> Result<String, SummarizeError> {
    let estimator = estimator_for_model(&model.id);
    let mut remaining = prompt;
    let optimistic_chunks =
        preflight_chunk_count(estimator, prompt, input_budget, steering.instructions)?;
    if optimistic_chunks > MAX_CHUNKS as u64 {
        return Err(SummarizeError::Failed(format!(
            "history needs at least {optimistic_chunks} fragments; maximum is {MAX_CHUNKS}"
        )));
    }
    let mut summary = None::<String>;
    for chunk in 0..MAX_CHUNKS {
        let mut system_prompt = build_system_prompt(
            summary.as_deref().or(steering.previous_summary),
            steering.instructions,
        );
        // The rendered transcript is split, not truncated. Even one oversized
        // user message can cross a boundary; every byte is consumed in order.
        if chunk > 0 || request_tokens(estimator, &system_prompt, remaining) > input_budget {
            system_prompt.push_str(
                "\nThe conversation is supplied in consecutive fragments. This fragment may \
                 start or end within a message. Merge it into the anchored summary, preserving \
                 relevant facts from earlier fragments; do not treat a fragment as a complete history.",
            );
        }
        let end = fitting_prefix(estimator, &system_prompt, remaining, input_budget);
        if end == 0 {
            return Err(SummarizeError::Failed(format!(
                "summary instructions and anchor leave no room for history within {input_budget} tokens"
            )));
        }
        let user_prompt = remaining[..end].to_owned();
        tracing::debug!(
            model = %model.id,
            provider = ?model.provider,
            chunk = chunk + 1,
            input_budget,
            input_tokens = request_tokens(estimator, &system_prompt, &user_prompt),
            "summarizing bounded history fragment"
        );
        let next = deps
            .summarizer
            .summarize(SummarizeRequest {
                system_prompt,
                user_prompt,
                model: model.id.clone(),
                provider: model.provider.clone(),
            })
            .await?;
        if next.trim().is_empty() {
            return Err(SummarizeError::Empty);
        }
        remaining = &remaining[end..];
        if remaining.is_empty() {
            return Ok(next);
        }
        summary = Some(next);
    }
    // Do not return a partial anchor: the caller must not persist a summary
    // that silently drops the unprocessed suffix of the conversation.
    Err(SummarizeError::Failed(format!(
        "history requires more than {MAX_CHUNKS} bounded summary calls"
    )))
}

/// Returns an optimistic lower bound on the number of requests needed.
///
/// If that empty request leaves `capacity` estimated tokens for history,
/// let `B` be the serialized size of its empty user-message envelope. A
/// fragment with `n` raw bytes serializes to at least `B + n` bytes, so its
/// incremental estimate is `floor((B % 4 + n) / 4)`. Therefore any `n >
/// 4 * capacity + 3` exceeds the remaining capacity, regardless of JSON
/// escaping. We deliberately use that upper bound (rather than assuming the
/// favourable remainder of `B`) when dividing the rendered prompt, so the
/// resulting count can only be lower than the true minimum. This does not
/// assume that an anchor grows; an initially long anchor may shrink on the
/// first successful call. The loop's MAX_CHUNKS guard remains necessary for
/// unpredictable anchor growth.
fn preflight_chunk_count(
    estimator: &dyn Estimator,
    prompt: &str,
    input_budget: u64,
    instructions: Option<&str>,
) -> Result<u64, SummarizeError> {
    let optimistic_system = build_system_prompt(None, instructions);
    let empty_request = request_tokens(estimator, &optimistic_system, "");
    let capacity = input_budget.checked_sub(empty_request).ok_or_else(|| {
        SummarizeError::Failed(format!(
            "summary instructions leave no room for history within {input_budget} tokens"
        ))
    })?;
    if capacity == 0 {
        return Err(SummarizeError::Failed(format!(
            "summary instructions leave no room for history within {input_budget} tokens"
        )));
    }

    let max_fragment_bytes = capacity.saturating_mul(4).saturating_add(3);
    let prompt_bytes = u64::try_from(prompt.len()).unwrap_or(u64::MAX);
    Ok(prompt_bytes.div_ceil(max_fragment_bytes))
}

fn request_tokens(estimator: &dyn Estimator, system_prompt: &str, user_prompt: &str) -> u64 {
    // Count the actual user-message shape, including JSON escaping and framing,
    // rather than estimating raw text alone (quotes/newlines can double in JSON).
    let message = AgentMessage::User {
        content: vec![ContentBlock::Text {
            text: user_prompt.to_owned(),
        }],
        timestamp: i64::MAX,
    };
    estimator
        .text(system_prompt)
        .saturating_add(estimator.message(&message))
        .saturating_add(REQUEST_OVERHEAD_TOKENS)
}

fn fitting_prefix(estimator: &dyn Estimator, system: &str, text: &str, budget: u64) -> usize {
    if request_tokens(estimator, system, text) <= budget {
        return text.len();
    }
    let mut low = 0;
    let mut high = text.len();
    while low < high {
        let midpoint = low + (high - low).div_ceil(2);
        let mut end = midpoint;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if request_tokens(estimator, system, &text[..end]) <= budget {
            low = midpoint;
        } else {
            high = midpoint - 1;
        }
    }
    while !text.is_char_boundary(low) {
        low -= 1;
    }
    low
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::estimate::HeuristicEstimator;

    #[test]
    fn fitting_prefix_accounts_for_escaping_and_never_splits_unicode() {
        let text = "東京🙂\\\"\n".repeat(300);
        let end = fitting_prefix(&HeuristicEstimator, "summary", &text, 300);
        assert!(end > 0 && end < text.len());
        assert!(text.is_char_boundary(end));
        assert!(request_tokens(&HeuristicEstimator, "summary", &text[..end]) <= 300);
        let next = end + text[end..].chars().next().unwrap().len_utf8();
        assert!(request_tokens(&HeuristicEstimator, "summary", &text[..next]) > 300);
    }

    #[test]
    fn preflight_boundary_is_conservative_at_thirty_two_fragments() {
        let estimator = &HeuristicEstimator;
        let system = build_system_prompt(None, None);
        let empty_request = request_tokens(estimator, &system, "");
        let capacity = 7;
        let per_fragment = capacity * 4 + 3;
        let boundary = "x".repeat(per_fragment as usize * MAX_CHUNKS);
        assert_eq!(
            preflight_chunk_count(estimator, &boundary, empty_request + capacity, None),
            Ok(MAX_CHUNKS as u64)
        );
        let impossible = format!("{boundary}x");
        assert_eq!(
            preflight_chunk_count(estimator, &impossible, empty_request + capacity, None),
            Ok(MAX_CHUNKS as u64 + 1)
        );
    }

    #[test]
    fn an_oversized_anchor_leaves_no_prefix() {
        assert_eq!(
            fitting_prefix(&HeuristicEstimator, &"x".repeat(4000), "history", 100),
            0
        );
    }
}
