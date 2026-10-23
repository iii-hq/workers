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

pub struct CompactParams {
    pub messages: Vec<Value>,
    pub model_id: String,
    pub provider: Option<String>,
    pub previous_summary: Option<String>,
    /// One-shot guidance for the summariser (the text after `/compact`).
    pub instructions: Option<String>,
    pub lease_key: String,
}

/// `context::compact`'s response, discriminated on `status`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CompactOutput {
    Ok {
        summary: String,
        /// Index into the request `messages` where the verbatim tail begins;
        /// `None` when everything was summarised.
        #[serde(default)]
        tail_start_index: Option<usize>,
        tokens_before: u64,
        #[serde(default)]
        tokens_after: Option<u64>,
    },
    Busy,
    Empty,
    Overflow,
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
        if let Some(allow) = params.allow_prune {
            options["allow_prune"] = json!(allow);
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

    /// Summarise the head of `messages` under the session lease. No inline
    /// limits: the router resolves the model's real output allowance, which
    /// sizes every summariser fragment to what the provider accepts.
    pub async fn compact(&self, params: CompactParams) -> Result<CompactOutput, String> {
        let mut model = json!({ "id": params.model_id });
        if let Some(p) = &params.provider {
            model["provider"] = json!(p);
        }
        let mut options = json!({ "lease_key": params.lease_key });
        if let Some(s) = &params.previous_summary {
            options["previous_summary"] = json!(s);
        }
        if let Some(i) = &params.instructions {
            options["instructions"] = json!(i);
        }
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "context::compact".into(),
                payload: json!({
                    "messages": params.messages,
                    "model": model,
                    "options": options,
                }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await;
        match resp {
            Ok(v) => serde_json::from_value::<CompactOutput>(v)
                .map_err(|e| format!("context::compact parse: {e}")),
            Err(e) => Err(format!("context::compact: {e}")),
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
