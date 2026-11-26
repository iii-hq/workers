//! Required `context-manager` client: context assembly and final token
//! preflight both fail closed so the harness never bypasses provider budgets.

use std::collections::BTreeMap;
use std::sync::Arc;

use iii_sdk::IIIClient;
use iii_sdk::protocol::CallRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::types::message::AgentMessage;
use crate::types::model::{AgentFunction, ThinkingLevel};

/// The budgeted context returned by `context::assemble`.
#[derive(Debug, Clone, Deserialize)]
pub struct AssembleOutput {
    #[serde(default)]
    pub input_indices: Option<Vec<usize>>,
    #[serde(default)]
    pub(crate) summary_origin: Option<crate::clients::router::generation_capture::AssemblySummary>,
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
    /// The `compaction_budget` context-manager applied (chat-projects D160):
    /// `None` when the field is missing (a context-manager older than 1.3.0,
    /// which ignored the budget), `Some(None)` when it applied none.
    #[serde(default, deserialize_with = "present")]
    pub compaction_budget: Option<Option<u64>>,
    /// The compaction this assembly tried, when it tried one (1.3.0).
    #[serde(default)]
    pub compaction: Option<AssembleCompaction>,
}

/// A field that may be `null`, told apart from a missing one.
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<u64>>, D::Error> {
    Option::<u64>::deserialize(d).map(Some)
}

/// `context::assemble`'s `compaction` (D160).
#[derive(Debug, Clone, Deserialize)]
pub struct AssembleCompaction {
    /// `budget` (the harness's `compaction_budget`) or `usable` (the model's
    /// own limit).
    pub trigger: String,
    /// `compacted`, `failed`, `busy` or `skipped`.
    pub status: String,
    #[serde(default)]
    pub failure: Option<CompactFailure>,
}

impl AssembleCompaction {
    /// The budget fired the compaction, not the model's limit.
    pub fn by_budget(&self) -> bool {
        self.trigger == "budget"
    }

    /// An attempt that should not be repeated in this turn.
    pub fn failed_or_busy(&self) -> bool {
        matches!(self.status.as_str(), "failed" | "busy")
    }
}

/// Why the summariser failed (context-manager 1.3.0, D162).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompactFailure {
    /// `usage_limit`, `rate_limited`, `input_too_large`, `timeout`,
    /// `unavailable` or `other`.
    pub kind: String,
    pub message: String,
    /// A usage limit's details (D147).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_details: Option<crate::types::message::ErrorDetails>,
}

impl CompactFailure {
    /// The failure a `context::compact` error carries in its message, when
    /// it carries one: a JSON object that is the failure, or holds it under
    /// `failure`.
    /// A `context/overflow` message ends with `; compaction failed: ` and
    /// the failure.
    pub fn from_error(error: &str) -> Option<Self> {
        let json = match error.split_once(OVERFLOW_FAILURE_MARKER) {
            Some((_, json)) => json,
            None => &error[error.find('{')?..],
        };
        let value: Value = serde_json::from_str(json.trim()).ok()?;
        let failure = value.get("failure").cloned().unwrap_or(value);
        serde_json::from_value(failure).ok()
    }

    /// The account reached its plan's usage limit (the kind, or the router's
    /// details on another kind).
    pub fn is_usage_limit(&self) -> bool {
        self.kind == "usage_limit"
            || crate::types::message::ErrorDetails::usage_limit(self.error_details.as_ref())
                .is_some()
    }
}

/// What precedes the JSON failure at the end of a `context/overflow`
/// message whose inline compaction failed (context-manager 1.3.0).
const OVERFLOW_FAILURE_MARKER: &str = "; compaction failed: ";

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
    /// The `response_format` the turn sends on `router::chat`, so the
    /// provider's count renders it where its template does.
    pub response_format: Option<Value>,
    pub tools: Vec<AgentFunction>,
    pub request_overhead_tokens: u64,
    /// `false` keeps earlier function results as they were sent (no aged
    /// pruning); `None` leaves context-manager's default.
    pub allow_prune: Option<bool>,
    /// Ask for `input_indices` / `summary_origin`: the generation capture
    /// witness only, never a production assembly.
    pub provenance: bool,
    /// The autocompact budget (D160): gate, prune and compaction use
    /// `min(usable, budget)`; emergency reduction stays at `usable`.
    pub compaction_budget: Option<u64>,
    /// What an inline compaction's summary keeps (the global setting's text).
    pub compaction_instructions: Option<String>,
    /// The turn's provider options, for the summariser's `router::chat` (the
    /// conversation's Claude Code account, D161).
    pub provider_options: Option<Value>,
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
    /// Forwarded to the summariser's `router::chat` (D161).
    pub provider_options: Option<Value>,
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
    /// The summariser failed; `failure` says why (1.3.0).
    Overflow {
        #[serde(default)]
        failure: Option<CompactFailure>,
    },
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
        let resp = self
            .iii
            .call(CallRequest {
                function_id: "context::assemble".into(),
                payload: assemble_payload(params),
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
        if let Some(provider_options) = params.provider_options.as_ref().filter(|v| v.is_object()) {
            options["provider_options"] = provider_options.clone();
        }
        let resp = self
            .iii
            .call(CallRequest {
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
            .call(CallRequest {
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

/// The `context::assemble` request for `params`.
fn assemble_payload(params: AssembleParams) -> Value {
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
    if let Some(format) = params.response_format {
        options["response_format"] = format;
    }
    if let Some(allow) = params.allow_prune {
        options["allow_prune"] = json!(allow);
    }
    if params.provenance {
        options["provenance"] = json!(true);
    }
    if let Some(budget) = params.compaction_budget {
        options["compaction_budget"] = json!(budget);
    }
    if let Some(instructions) = &params.compaction_instructions {
        options["compaction_instructions"] = json!(instructions);
    }
    // context-manager takes an object only (namespaced by provider id).
    if let Some(provider_options) = params.provider_options.filter(Value::is_object) {
        options["provider_options"] = provider_options;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assemble_carries_the_turns_level_and_format_for_the_provider_count() {
        let params = |response_format| AssembleParams {
            messages: vec![],
            model_id: "m".into(),
            provider: None,
            system_prompt: None,
            parts: None,
            previous_summary: None,
            lease_key: "s".into(),
            thinking_level: Some(ThinkingLevel::Off),
            response_format,
            tools: vec![],
            request_overhead_tokens: 0,
            allow_prune: None,
            provenance: false,
            compaction_budget: None,
            compaction_instructions: None,
            provider_options: None,
        };
        let payload = assemble_payload(params(Some(json!({ "type": "json" }))));
        assert!(payload["options"].get("provenance").is_none(), "{payload}");
        assert_eq!(payload["options"]["thinking_level"], "off");
        assert_eq!(
            payload["options"]["response_format"],
            json!({ "type": "json" })
        );

        let payload = assemble_payload(params(None));
        assert!(
            payload["options"].get("response_format").is_none(),
            "{payload}"
        );
        for key in [
            "compaction_budget",
            "compaction_instructions",
            "provider_options",
        ] {
            assert!(payload["options"].get(key).is_none(), "{payload}");
        }
    }

    /// The autocompact options ride `options` (context-manager 1.3.0).
    #[test]
    fn assemble_carries_the_autocompact_budget_instructions_and_account() {
        let payload = assemble_payload(AssembleParams {
            messages: vec![],
            model_id: "m".into(),
            provider: None,
            system_prompt: None,
            parts: None,
            previous_summary: None,
            lease_key: "s".into(),
            thinking_level: None,
            response_format: None,
            tools: vec![],
            request_overhead_tokens: 0,
            allow_prune: None,
            provenance: false,
            compaction_budget: Some(180_000),
            compaction_instructions: Some("Keep the PR numbers.".into()),
            provider_options: Some(json!({ "claude-code": { "account": "work" } })),
        });
        assert_eq!(payload["options"]["compaction_budget"], 180_000);
        assert_eq!(
            payload["options"]["compaction_instructions"],
            "Keep the PR numbers."
        );
        assert_eq!(
            payload["options"]["provider_options"]["claude-code"]["account"],
            "work"
        );
    }

    /// A missing echo is an older context-manager; `null` is no budget.
    #[test]
    fn the_budget_echo_tells_an_older_context_manager_apart() {
        let base = json!({ "token_count": 1, "usable": 2, "effective_max_output_tokens": 3 });
        let old: AssembleOutput = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(old.compaction_budget, None);
        assert!(old.compaction.is_none());
        let mut none = base.clone();
        none["compaction_budget"] = Value::Null;
        let none: AssembleOutput = serde_json::from_value(none).unwrap();
        assert_eq!(none.compaction_budget, Some(None));
        let mut applied = base;
        applied["compaction_budget"] = json!(180_000);
        applied["compaction"] = json!({
            "trigger": "budget", "status": "failed",
            "failure": { "kind": "usage_limit", "message": "limit",
                "error_details": { "usage_limit": true, "window": "week", "resets_at_ms": 9 } }
        });
        let applied: AssembleOutput = serde_json::from_value(applied).unwrap();
        assert_eq!(applied.compaction_budget, Some(Some(180_000)));
        let compaction = applied.compaction.unwrap();
        assert!(compaction.by_budget() && compaction.failed_or_busy());
        let failure = compaction.failure.unwrap();
        assert_eq!(failure.kind, "usage_limit");
        assert_eq!(
            failure
                .error_details
                .and_then(|details| details.resets_at_ms),
            Some(9)
        );
    }

    #[test]
    fn an_overflow_carries_its_failure_when_there_is_one() {
        let old: CompactOutput = serde_json::from_value(json!({ "status": "overflow" })).unwrap();
        assert!(matches!(old, CompactOutput::Overflow { failure: None }));
        let new: CompactOutput = serde_json::from_value(json!({
            "status": "overflow", "failure": { "kind": "input_too_large", "message": "too big" }
        }))
        .unwrap();
        let CompactOutput::Overflow {
            failure: Some(failure),
        } = new
        else {
            panic!("a failure")
        };
        assert_eq!(failure.kind, "input_too_large");
        assert_eq!(
            CompactFailure::from_error(
                r#"context::compact: handler: {"failure":{"kind":"timeout","message":"slow"}}"#
            )
            .map(|failure| failure.kind),
            Some("timeout".into())
        );
        assert_eq!(CompactFailure::from_error("context::compact: boom"), None);
        let overflow = CompactFailure::from_error(
            r#"context::assemble: remote error (context/overflow): context/overflow: assembled context requires 101 tokens but usable budget is 100; compaction failed: {"kind":"usage_limit","message":"limit reached","error_details":{"usage_limit":true,"window":"five_hour","resets_at_ms":1700000000000}}"#,
        )
        .expect("the overflow's failure");
        assert!(overflow.is_usage_limit());
        assert_eq!(
            overflow
                .error_details
                .and_then(|details| details.resets_at_ms),
            Some(1_700_000_000_000)
        );
    }
}
