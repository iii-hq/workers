//! `context::count-tokens` — estimate token usage for a set of
//! messages, optionally including invocation schemas and a system
//! prompt, vs a model (context-manager.md § context::count-tokens).
//!
//! Pure and router-free: the model selects a tokenizer (v1 always
//! falls back to the generic heuristic, reported in `estimator`), so
//! cost-sensitive callers can run this with no `llm-router` installed.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::estimate::{estimate_by_role, estimate_messages, estimator_for_model};
use crate::error::ContextError;
use crate::ports::Deps;
use crate::types::{AgentFunction, AgentMessage, ByRoleTokens, EstimatorName, ModelInput};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Messages to estimate, oldest first.
    pub messages: Option<Vec<AgentMessage>>,
    /// Counted on top of the messages when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Invocation schema(s) to include in the estimate (typically the
    /// single `agent_trigger` entry).
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Named auxiliary texts to count individually with the same
    /// estimator (for example the segments a system prompt is built
    /// from). Counted in `by_part` only; never added to `tokens`.
    #[serde(default)]
    pub parts: Option<BTreeMap<String, String>>,
    /// Tokenizer selection; falls back to a generic estimator.
    pub model: ModelInput,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    /// Total estimate: messages + system prompt + tools.
    pub tokens: u64,
    /// Breakdown of the message tokens by role (system prompt and
    /// tools are not part of any role bucket).
    pub by_role: Option<ByRoleTokens>,
    /// The `tools` share of `tokens`; present when the request carried
    /// `tools`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools_tokens: Option<u64>,
    /// Per-part estimates for the request's `parts`, keyed by the
    /// caller's names; present when the request carried `parts`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_part: Option<BTreeMap<String, u64>>,
    pub estimator: EstimatorName,
}

pub async fn handle(
    _deps: &Deps,
    req: CountTokensRequest,
) -> Result<CountTokensResponse, ContextError> {
    let messages = req
        .messages
        .ok_or_else(|| ContextError::InvalidRequest("messages is required".into()))?;

    let estimator = estimator_for_model(&req.model.id);

    let mut tokens = estimate_messages(estimator, &messages);
    if let Some(system_prompt) = &req.system_prompt {
        tokens += estimator.text(system_prompt);
    }
    let tools_tokens = req.tools.as_ref().map(|tools| {
        tools
            .iter()
            .map(|tool| estimator.function(tool))
            .sum::<u64>()
    });
    tokens += tools_tokens.unwrap_or(0);

    let by_part = req.parts.as_ref().map(|parts| {
        parts
            .iter()
            .map(|(name, text)| (name.clone(), estimator.text(text)))
            .collect::<BTreeMap<String, u64>>()
    });

    let by_role = estimate_by_role(estimator, &messages);

    Ok(CountTokensResponse {
        tokens,
        by_role: Some(by_role.into()),
        tools_tokens,
        by_part,
        estimator: estimator.kind().into(),
    })
}

#[cfg(test)]
mod tests {
    use crate::core::estimate::{estimate_messages, Estimator, HeuristicEstimator};
    use crate::types::{AgentFunction, AgentMessage};
    use serde_json::json;

    /// Callers (the harness) substitute `context::assemble.token_count`
    /// for `count-tokens(...).tokens + overhead` when nothing mutated the
    /// request after assembly. That substitution is only valid while the
    /// two functions count the same inputs to the same number — pin it.
    #[test]
    fn count_tokens_and_assemble_count_the_same_context_identically() {
        let messages: Vec<AgentMessage> = vec![
            serde_json::from_value(json!({
                "role": "user",
                "content": [{ "type": "text", "text": "question" }],
                "timestamp": 1
            }))
            .unwrap(),
            serde_json::from_value(json!({
                "role": "assistant",
                "content": [{ "type": "text", "text": "answer" }],
                "stop_reason": "end", "model": "m", "provider": "p", "timestamp": 2
            }))
            .unwrap(),
        ];
        let tools: Vec<AgentFunction> = vec![serde_json::from_value(json!({
            "name": "agent_trigger",
            "description": "Run an agent function",
            "parameters": { "type": "object" }
        }))
        .unwrap()];
        let prompt = "system prompt";
        let overhead = 37u64;
        let estimator = HeuristicEstimator;

        // count-tokens' arithmetic (handle() above), plus the overhead the
        // harness adds on top.
        let mut count_tokens_total = estimate_messages(&estimator, &messages);
        count_tokens_total += estimator.text(prompt);
        for tool in &tools {
            count_tokens_total += estimator.function(tool);
        }
        count_tokens_total += overhead;

        // assemble's arithmetic (its overhead-inclusive count).
        let assemble_total = crate::functions::assemble::count_context_for_tests(
            &messages, prompt, &tools, overhead, &estimator,
        );

        assert_eq!(assemble_total, count_tokens_total);
    }
}
