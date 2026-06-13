//! `context::count_tokens` — estimate token usage for a set of
//! messages, optionally including invocation schemas and a system
//! prompt, vs a model (context-manager.md § context::count_tokens).
//!
//! Pure and router-free: the model selects a tokenizer (v1 always
//! falls back to the generic heuristic, reported in `estimator`), so
//! cost-sensitive callers can run this with no `llm-router` installed.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::estimate::{estimate_by_role, estimate_messages, estimator_for_model};
use crate::error::ContextError;
use crate::ports::Deps;
use crate::types::{AgentFunction, AgentMessage, ModelInput};

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
    /// Tokenizer selection; falls back to a generic estimator.
    pub model: ModelInput,
}

/// Per-role token breakdown of the `messages` array.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ByRoleTokens {
    pub user: u64,
    pub assistant: u64,
    pub function_result: u64,
    pub custom: u64,
}

/// Which estimator produced the count.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EstimatorName {
    Tokenizer,
    Heuristic,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    /// Total estimate: messages + system prompt + tools.
    pub tokens: u64,
    /// Breakdown of the message tokens by role (system prompt and
    /// tools are not part of any role bucket).
    pub by_role: Option<ByRoleTokens>,
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
    for tool in req.tools.iter().flatten() {
        tokens += estimator.function(tool);
    }

    let by_role = estimate_by_role(estimator, &messages);

    Ok(CountTokensResponse {
        tokens,
        by_role: Some(ByRoleTokens {
            user: by_role.user,
            assistant: by_role.assistant,
            function_result: by_role.function_result,
            custom: by_role.custom,
        }),
        estimator: match estimator.kind() {
            crate::core::estimate::EstimatorKind::Tokenizer => EstimatorName::Tokenizer,
            crate::core::estimate::EstimatorKind::Heuristic => EstimatorName::Heuristic,
        },
    })
}
