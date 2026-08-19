//! Required `context-manager` client: context assembly and final token
//! preflight both fail closed so the harness never bypasses provider budgets.

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::types::message::AgentMessage;
use crate::types::model::{AgentFunction, ThinkingLevel};

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
}

pub struct AssembleParams {
    pub messages: Vec<Value>,
    pub model_id: String,
    pub provider: Option<String>,
    pub system_prompt: Option<String>,
    pub previous_summary: Option<String>,
    pub lease_key: String,
    pub thinking_level: Option<ThinkingLevel>,
    pub tools: Vec<AgentFunction>,
    pub request_overhead_tokens: u64,
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

    /// Assemble a model-ready context. The context manager is a required
    /// dependency: callers must never fall back to unbudgeted raw history.
    pub async fn assemble(&self, params: AssembleParams) -> Result<AssembleOutput, String> {
        let mut model = json!({ "id": params.model_id });
        if let Some(p) = &params.provider {
            model["provider"] = json!(p);
        }
        let mut options = json!({
            "lease_key": params.lease_key,
            "request_overhead_tokens": params.request_overhead_tokens,
        });
        if let Some(s) = &params.previous_summary {
            options["previous_summary"] = json!(s);
        }
        if let Some(tl) = &params.thinking_level {
            options["thinking_level"] = serde_json::to_value(tl).unwrap_or(Value::Null);
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

        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "context::assemble".into(),
                payload,
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await;

        match resp {
            Ok(v) => serde_json::from_value::<AssembleOutput>(v)
                .map_err(|e| format!("context::assemble parse: {e}")),
            Err(e) => Err(format!("context::assemble: {e}")),
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
