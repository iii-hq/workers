//! Required `context-manager` client: context assembly and final token
//! preflight both fail closed so the harness never bypasses provider budgets.

use std::collections::BTreeMap;
use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::types::message::AgentMessage;
use crate::types::model::{AgentFunction, ThinkingLevel};

/// context-manager's assembly function: every turn without an effective
/// context strategy assembles here.
pub const CONTEXT_MANAGER_ASSEMBLE: &str = "context::assemble";

/// The request body for [`ContextClient::assemble`]. Strategy workers read the
/// turn identity and root keys; context-manager ignores the keys it does not
/// know.
fn assemble_payload(params: &AssembleParams) -> Value {
    let mut model = json!({ "id": params.model_id });
    if let Some(p) = &params.provider {
        model["provider"] = json!(p);
    }
    let mut options = json!({
        "lease_key": params.lease_key,
        "request_overhead_tokens": params.request_overhead_tokens,
        "session_id": params.session_id,
        "turn_id": params.turn_id,
        "step": params.step,
        "filesystem_root": params.filesystem_root,
    });
    if let Some(s) = &params.previous_summary {
        options["previous_summary"] = json!(s);
    }
    if let Some(tl) = &params.thinking_level {
        options["thinking_level"] = serde_json::to_value(tl).unwrap_or(Value::Null);
    }
    if let Some(allow) = params.allow_prune {
        options["allow_prune"] = json!(allow);
    }
    if let Some(budget) = params.budget_tokens {
        options["budget_tokens"] = json!(budget);
    }
    let mut payload = json!({
        "messages": params.messages,
        "model": model,
        "options": options,
        "tools": params.tools,
    });
    if let Some(sp) = &params.system_prompt {
        payload["system_prompt"] = json!(sp);
    }
    if let Some(parts) = &params.parts {
        payload["parts"] = json!(parts);
    }
    payload
}

/// The budgeted context returned by `context::assemble`.
#[derive(Debug, Clone, Deserialize)]
pub struct AssembleOutput {
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub messages: Vec<AgentMessage>,
    pub token_count: u64,
    pub usable: u64,
    pub effective_max_output_tokens: u64,
    #[serde(default)]
    pub applied: Applied,
    /// Per-category estimates of `token_count`; `None` when the installed
    /// context-manager predates the breakdown response.
    #[serde(default)]
    pub breakdown: Option<AssembleBreakdown>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AssembleBreakdown {
    #[serde(default)]
    pub system_prompt_tokens: u64,
    #[serde(default)]
    pub tools_tokens: u64,
    #[serde(default)]
    pub by_role: ByRoleTokens,
    #[serde(default)]
    pub by_part: Option<BTreeMap<String, u64>>,
    #[serde(default)]
    pub estimator: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ByRoleTokens {
    #[serde(default)]
    pub user: u64,
    #[serde(default)]
    pub assistant: u64,
    #[serde(default)]
    pub function_result: u64,
    #[serde(default)]
    pub custom: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Applied {
    #[serde(default)]
    pub initial_token_count: u64,
    /// Function results were pruned or reduced to fit the budget.
    #[serde(default)]
    pub pruned: bool,
    #[serde(default)]
    pub compacted: bool,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tail_start_index: Option<i64>,
    #[serde(default)]
    pub tokens_before: Option<u64>,
    #[serde(default)]
    pub summarized_head_tokens: Option<u64>,
    /// A context strategy ended the run on purpose: sent together with
    /// `token_count > usable`, it fails the turn with this text
    /// (`harness.context_budget_stop`) instead of the overflow message.
    #[serde(default)]
    pub strategy_stop: Option<String>,
}

pub struct AssembleParams {
    pub messages: Vec<Value>,
    pub model_id: String,
    pub provider: Option<String>,
    pub system_prompt: Option<String>,
    pub parts: Option<BTreeMap<String, String>>,
    pub previous_summary: Option<String>,
    pub lease_key: String,
    pub thinking_level: Option<ThinkingLevel>,
    pub tools: Vec<AgentFunction>,
    pub request_overhead_tokens: u64,
    /// `false` keeps earlier function results as they were sent (no aged
    /// pruning); `None` leaves context-manager's default.
    pub allow_prune: Option<bool>,
    /// [`CONTEXT_MANAGER_ASSEMBLE`], or `context-<name>::assemble` for a turn
    /// whose context strategy is effective.
    pub function_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub step: u64,
    /// The turn's `fs_scope.root`; sent as `null` when the turn is unscoped.
    pub filesystem_root: Option<String>,
    /// The effective strategy's per-session budget.
    pub budget_tokens: Option<u64>,
}

pub struct CountTokensParams {
    pub messages: Vec<Value>,
    pub model_id: String,
    pub provider: Option<String>,
    pub system_prompt: Option<String>,
    pub tools: Vec<AgentFunction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CountTokensOutput {
    pub tokens: u64,
    /// The message tokens by role (system prompt and tools in no bucket).
    #[serde(default)]
    pub by_role: Option<ByRoleTokens>,
}

#[derive(Clone)]
pub struct ContextClient {
    iii: Arc<IIIClient>,
    timeout_ms: u64,
}

impl ContextClient {
    pub fn new(iii: Arc<IIIClient>, timeout_ms: u64) -> Self {
        Self { iii, timeout_ms }
    }

    /// Assemble a model-ready context with `params.function_id`: the context
    /// manager's `context::assemble`, or the session's strategy function. The
    /// assembly function is a required dependency: callers must never fall
    /// back to unbudgeted raw history.
    pub async fn assemble(&self, params: AssembleParams) -> Result<AssembleOutput, String> {
        let payload = assemble_payload(&params);
        let function_id = params.function_id;
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: function_id.clone(),
                payload,
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await;

        match resp {
            Ok(v) => serde_json::from_value::<AssembleOutput>(v)
                .map_err(|e| format!("{function_id} parse: {e}")),
            Err(e) => Err(format!("{function_id}: {e}")),
        }
    }

    /// Count the final model-facing messages, prompt, and complete tool list.
    pub async fn count_tokens(
        &self,
        params: CountTokensParams,
    ) -> Result<CountTokensOutput, String> {
        let mut model = json!({ "id": params.model_id });
        if let Some(p) = &params.provider {
            model["provider"] = json!(p);
        }
        let mut payload = json!({
            "messages": params.messages,
            "model": model,
            "tools": params.tools,
        });
        if let Some(sp) = &params.system_prompt {
            payload["system_prompt"] = json!(sp);
        }

        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "context::count-tokens".into(),
                payload,
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await;

        match resp {
            Ok(v) => serde_json::from_value::<CountTokensOutput>(v)
                .map_err(|e| format!("context::count-tokens parse: {e}")),
            Err(e) => Err(format!("context::count-tokens: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(filesystem_root: Option<&str>, budget_tokens: Option<u64>) -> AssembleParams {
        AssembleParams {
            messages: vec![json!({ "role": "user", "content": [], "timestamp": 1 })],
            model_id: "m".into(),
            provider: Some("p".into()),
            system_prompt: Some("sys".into()),
            parts: None,
            previous_summary: None,
            lease_key: "s_1".into(),
            thinking_level: None,
            tools: Vec::new(),
            request_overhead_tokens: 64,
            allow_prune: Some(false),
            function_id: "context-clm::assemble".into(),
            session_id: "s_1".into(),
            turn_id: "t_1".into(),
            step: 3,
            filesystem_root: filesystem_root.map(str::to_string),
            budget_tokens,
        }
    }

    #[test]
    fn assemble_payload_carries_the_turn_identity_root_and_strategy_budget() {
        let payload = assemble_payload(&params(Some("/work"), Some(16_384)));
        assert_eq!(
            payload["options"],
            json!({
                "lease_key": "s_1",
                "request_overhead_tokens": 64,
                "session_id": "s_1",
                "turn_id": "t_1",
                "step": 3,
                "filesystem_root": "/work",
                "allow_prune": false,
                "budget_tokens": 16384
            })
        );
        assert_eq!(payload["model"], json!({ "id": "m", "provider": "p" }));
        assert_eq!(payload["system_prompt"], "sys");
        assert_eq!(payload["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn assemble_payload_sends_a_null_root_and_no_budget_when_unset() {
        let payload = assemble_payload(&params(None, None));
        let options = payload["options"].as_object().unwrap();
        assert_eq!(options.get("filesystem_root"), Some(&Value::Null));
        assert!(!options.contains_key("budget_tokens"));
    }
}
