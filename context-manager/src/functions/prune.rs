//! `context::prune` — replace eligible old function outputs with
//! placeholders without summarising (context-manager.md §
//! context::prune). The cheaper first pass before compaction; safe to
//! expose to agents (no LLM call, no state).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::estimate::estimator_for_model;
use crate::core::prune::{prune as run_prune, PruneParams};
use crate::error::ContextError;
use crate::ports::Deps;
use crate::types::{AgentMessage, ModelInput};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PruneOptions {
    /// Newest function-output tokens kept by normal pruning (default 40000).
    #[serde(default)]
    pub protect_recent_tokens: Option<u64>,
    /// Prune outputs after this many subsequent user turns. `null` uses
    /// worker config (default 0); `0` disables age-based pruning.
    #[serde(default)]
    pub decay_user_turns: Option<usize>,
    /// Most recent user turns exempt from normal pruning. `null` uses
    /// worker config (default 2); `0` disables only this exemption.
    #[serde(default)]
    pub protected_user_turns: Option<usize>,
    /// Skip the pass entirely when it would free less (default 20000).
    #[serde(default)]
    pub min_free_tokens: Option<u64>,
    /// Per-output verbosity threshold in chars (default 2000). Outputs
    /// at or under it can still be pruned after the configured decay age.
    #[serde(default)]
    pub max_output_chars: Option<usize>,
    /// `function_id`s whose outputs are never pruned.
    #[serde(default)]
    pub protected_functions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PruneRequest {
    /// Messages to prune, oldest first.
    pub messages: Option<Vec<AgentMessage>>,
    /// Only used for token math; the heuristic estimator needs no
    /// model, so this is optional.
    #[serde(default)]
    pub model: Option<ModelInput>,
    #[serde(default)]
    pub options: Option<PruneOptions>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PruneResponse {
    /// Same array with pruned outputs replaced by placeholders — no
    /// message or block is ever removed.
    pub messages: Vec<AgentMessage>,
    /// Estimated tokens freed.
    pub pruned_tokens: u64,
    /// Outputs replaced with placeholders.
    pub pruned_parts: u64,
    /// Prunable outputs examined.
    pub scanned_parts: u64,
}

pub async fn handle(deps: &Deps, req: PruneRequest) -> Result<PruneResponse, ContextError> {
    let mut messages = req
        .messages
        .ok_or_else(|| ContextError::InvalidRequest("messages is required".into()))?;

    let config = deps.config().await;
    let options = req.options.unwrap_or_default();
    let params = PruneParams {
        protect_recent_tokens: options
            .protect_recent_tokens
            .unwrap_or(config.protect_recent_tokens),
        decay_user_turns: options.decay_user_turns.unwrap_or(config.decay_user_turns),
        protected_user_turns: options
            .protected_user_turns
            .unwrap_or(config.protected_user_turns),
        min_free_tokens: options.min_free_tokens.unwrap_or(config.min_free_tokens),
        max_output_chars: options.max_output_chars.unwrap_or(config.max_output_chars),
        protected_functions: options.protected_functions.unwrap_or_default(),
    };

    let model_id = req.model.as_ref().map(|m| m.id.as_str()).unwrap_or("");
    let estimator = estimator_for_model(model_id);

    let stats = run_prune(&mut messages, &params, estimator);

    Ok(PruneResponse {
        messages,
        pruned_tokens: stats.pruned_tokens,
        pruned_parts: stats.pruned_parts,
        scanned_parts: stats.scanned_parts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_options_schema_exposes_decay_controls() {
        let schema = schemars::schema_for!(PruneOptions);
        let properties = schema.schema.object.expect("object schema").properties;
        assert!(properties.contains_key("decay_user_turns"));
        assert!(properties.contains_key("protected_user_turns"));

        let explicit_zero: PruneOptions = serde_json::from_value(serde_json::json!({
            "decay_user_turns": 0,
            "protected_user_turns": 0
        }))
        .unwrap();
        assert_eq!(explicit_zero.decay_user_turns, Some(0));
        assert_eq!(explicit_zero.protected_user_turns, Some(0));

        let inherited: PruneOptions = serde_json::from_value(serde_json::json!({
            "decay_user_turns": null,
            "protected_user_turns": null
        }))
        .unwrap();
        assert_eq!(inherited.decay_user_turns, None);
        assert_eq!(inherited.protected_user_turns, None);
    }
}
