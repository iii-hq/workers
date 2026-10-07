//! Read-only context policy shared by the Harness and console previews.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;

pub const ID: &str = "harness::context-policy";
pub const DESC: &str =
    "Return the Harness context-pruning policy for a destination model without making a model request.";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContextPolicyRequest {
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContextPolicyResponse {
    /// Whether aged function results may be pruned for this model.
    pub allow_prune: bool,
}

pub async fn handle(
    _deps: &Deps,
    req: ContextPolicyRequest,
) -> Result<ContextPolicyResponse, HarnessError> {
    Ok(ContextPolicyResponse {
        allow_prune: !crate::window::binds_thinking(&req.model),
    })
}

#[cfg(test)]
mod tests {
    use super::{handle, ContextPolicyRequest};

    #[tokio::test]
    async fn handler_delegates_to_the_shared_binding_predicate() {
        let deps = crate::functions::subscribe::tests::disconnected_deps();
        for (model, allow_prune) in [("claude-opus-5-5", false), ("claude-mythos-5-1", true)] {
            let response = handle(
                &deps,
                ContextPolicyRequest {
                    model: model.to_string(),
                },
            )
            .await
            .expect("context policy handler should succeed");
            assert_eq!(
                response.allow_prune, allow_prune,
                "policy drift for {model}"
            );
        }
    }

    #[test]
    fn policy_matches_the_shared_binding_predicate_for_supported_and_common_ids() {
        for (model, allow_prune) in [
            ("claude-opus-5-5", false),
            ("claude-opus-5-5-20260901", false),
            ("anthropic.claude-fable-5-1", false),
            ("claude-code/claude-opus-5-5", false),
            ("claude-sonnet-5-5", false),
            ("claude-mythos-5-1", true),
            ("openai::small", true),
        ] {
            assert_eq!(
                !crate::window::binds_thinking(model),
                allow_prune,
                "policy drift for {model}"
            );
        }
    }
}
