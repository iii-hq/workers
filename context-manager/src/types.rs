//! Cross-cutting wire contracts shared by the agentic worker family.
//!
//! These shapes mirror the spec's "Cross-cutting contracts"
//! (tech-specs/2026-06-agentic/README.md): `ContentBlock`,
//! `AgentMessage`, `AgentFunction`, `Model`, `ThinkingLevel` — plus this
//! worker's `ModelInput` (context-manager.md § Model input). Serde
//! renames keep the wire format byte-compatible with the TypeScript
//! definitions (and with session-manager's Rust copy).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Message role discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    FunctionResult,
    Custom,
}

/// Why an assistant message stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    End,
    Length,
    FunctionCall,
    Aborted,
    Error,
}

/// Coarse error classification carried by failed assistant messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    AuthExpired,
    RateLimited,
    ContextOverflow,
    Transient,
    Permanent,
}

/// Token / cost accounting reported by providers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Usage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// The atomic unit of message content. A message's `content` is an
/// ordered array of these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        /// MIME type, e.g. `image/png`.
        mime: String,
        /// Base64-encoded image bytes.
        data: String,
    },
    Thinking {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    FunctionCall {
        /// Unique per call, echoed by the result.
        id: String,
        /// The iii function id to invoke.
        function_id: String,
        /// Model-produced arguments (JSON).
        #[serde(default)]
        arguments: Value,
    },
    FunctionResult {
        function_call_id: String,
        content: Vec<ContentBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// The canonical transcript message union, discriminated by `role`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum AgentMessage {
    User {
        content: Vec<ContentBlock>,
        /// Milliseconds since epoch.
        timestamp: i64,
    },
    Assistant {
        content: Vec<ContentBlock>,
        stop_reason: StopReason,
        /// Provider's raw finish reason, passed through untouched.
        #[serde(skip_serializing_if = "Option::is_none")]
        native_stop_reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_kind: Option<ErrorKind>,
        /// Report-and-continue notices (e.g. dropped unsupported param).
        #[serde(skip_serializing_if = "Option::is_none")]
        warnings: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        model: String,
        provider: String,
        /// Milliseconds since epoch.
        timestamp: i64,
    },
    FunctionResult {
        /// Echoes the `function_call` block id this result answers.
        function_call_id: String,
        function_id: String,
        content: Vec<ContentBlock>,
        /// Opaque structured payload kept alongside the rendered content.
        #[serde(default)]
        details: Value,
        #[serde(default)]
        is_error: bool,
        /// Milliseconds since epoch.
        timestamp: i64,
    },
    /// Escape hatch for app-specific transcript items (system notices,
    /// UI markers, attachments, ...). No provider wire mapping —
    /// `context::assemble` excludes these from the model-facing list.
    Custom {
        /// App-defined discriminator.
        custom_type: String,
        content: Vec<ContentBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        /// Milliseconds since epoch.
        timestamp: i64,
    },
}

impl AgentMessage {
    pub fn role(&self) -> Role {
        match self {
            AgentMessage::User { .. } => Role::User,
            AgentMessage::Assistant { .. } => Role::Assistant,
            AgentMessage::FunctionResult { .. } => Role::FunctionResult,
            AgentMessage::Custom { .. } => Role::Custom,
        }
    }

    pub fn content(&self) -> &[ContentBlock] {
        match self {
            AgentMessage::User { content, .. }
            | AgentMessage::Assistant { content, .. }
            | AgentMessage::FunctionResult { content, .. }
            | AgentMessage::Custom { content, .. } => content,
        }
    }

    pub fn set_content(&mut self, new_content: Vec<ContentBlock>) {
        match self {
            AgentMessage::User { content, .. }
            | AgentMessage::Assistant { content, .. }
            | AgentMessage::FunctionResult { content, .. }
            | AgentMessage::Custom { content, .. } => *content = new_content,
        }
    }

    /// True when any top-level content block is a `function_result` —
    /// used by tail selection to avoid cutting a result away from its
    /// call. Function-result content holds rendered output (text/image),
    /// not nested `function_result` blocks, so a top-level scan suffices.
    pub fn has_function_result_block(&self) -> bool {
        fn scan(blocks: &[ContentBlock]) -> bool {
            blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::FunctionResult { .. }))
        }
        scan(self.content())
    }
}

/// Reasoning-effort tier; levels map to provider-native knobs via
/// `Model.thinking_budgets`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

/// Token limits a caller may supply inline to stay fully standalone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelLimits {
    /// Total context window in tokens.
    pub context_window: u64,
    /// Maximum output tokens the model can produce per call.
    pub max_output_tokens: u64,
    /// Usable input budget when distinct from `context_window`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_limit: Option<u64>,
}

/// How callers identify the target model (context-manager.md § Model
/// input). Inline `limits` keep the call standalone; `id`/`provider`
/// alone defer limit resolution to `router::models::budget`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelInput {
    /// Model id, e.g. `claude-sonnet-4`.
    pub id: String,
    /// Provider id, e.g. `anthropic`; disambiguates when a model id
    /// exists on multiple providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Inline token limits; when present no router lookup happens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<ModelLimits>,
}

/// The capability record `llm-router` serves (README.md § Model
/// descriptor). Only the fields this worker reads are modelled;
/// unknown fields are ignored on deserialisation.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Model {
    /// Model id, e.g. `claude-sonnet-4`.
    pub id: String,
    /// Provider id, e.g. `anthropic`.
    #[serde(default)]
    pub provider: String,
    /// Total context window in tokens.
    pub context_window: u64,
    /// Maximum output tokens per call.
    pub max_output_tokens: u64,
    /// Usable input budget when distinct from `context_window`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_limit: Option<u64>,
    /// Reasoning-token budgets per thinking tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_budgets: Option<BTreeMap<ThinkingLevel, u64>>,
}

/// An invocation-surface schema entry (README.md § AgentFunction) —
/// accepted by `context::count-tokens` so callers can include the
/// `agent_trigger` schema in their estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentFunction {
    /// Invocation surface name (`agent_trigger`, `submit_result`, or a
    /// function id in native exposure mode).
    pub name: String,
    pub description: String,
    /// JSON Schema for the function parameters.
    #[serde(default)]
    pub parameters: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<ExecutionMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Parallel,
    Sequential,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn content_block_wire_shapes_match_spec() {
        let blocks: Vec<ContentBlock> = serde_json::from_value(json!([
            { "type": "text", "text": "hi" },
            { "type": "image", "mime": "image/png", "data": "aGk=" },
            { "type": "thinking", "text": "hmm", "signature": "sig" },
            { "type": "function_call", "id": "c1", "function_id": "f::g", "arguments": { "a": 1 } },
            { "type": "function_result", "function_call_id": "c1",
              "content": [{ "type": "text", "text": "out" }], "is_error": false }
        ]))
        .unwrap();
        assert_eq!(blocks.len(), 5);
        let round = serde_json::to_value(&blocks).unwrap();
        assert_eq!(round[0], json!({ "type": "text", "text": "hi" }));
        assert_eq!(round[3]["function_id"], "f::g");
    }

    #[test]
    fn agent_message_roundtrip_all_roles() {
        let msgs: Vec<AgentMessage> = serde_json::from_value(json!([
            { "role": "user", "content": [{ "type": "text", "text": "q" }], "timestamp": 1 },
            { "role": "assistant", "content": [], "stop_reason": "end",
              "model": "m1", "provider": "p1", "timestamp": 2 },
            { "role": "function_result", "function_call_id": "c1", "function_id": "f::g",
              "content": [], "details": { "x": 1 }, "is_error": false, "timestamp": 3 },
            { "role": "custom", "custom_type": "notice", "content": [], "timestamp": 4 }
        ]))
        .unwrap();
        assert_eq!(msgs[0].role(), Role::User);
        assert_eq!(msgs[1].role(), Role::Assistant);
        assert_eq!(msgs[2].role(), Role::FunctionResult);
        assert_eq!(msgs[3].role(), Role::Custom);

        let round = serde_json::to_value(&msgs).unwrap();
        assert_eq!(round[1]["stop_reason"], "end");
        assert!(round[1].get("error_message").is_none());
        assert_eq!(round[2]["details"], json!({ "x": 1 }));
    }

    #[test]
    fn model_input_minimal_and_inline_forms() {
        let bare: ModelInput = serde_json::from_value(json!({ "id": "gpt-x" })).unwrap();
        assert_eq!(bare.id, "gpt-x");
        assert!(bare.provider.is_none() && bare.limits.is_none());

        let inline: ModelInput = serde_json::from_value(json!({
            "id": "m", "provider": "p",
            "limits": { "context_window": 1000, "max_output_tokens": 100, "input_limit": 900 }
        }))
        .unwrap();
        let limits = inline.limits.unwrap();
        assert_eq!(limits.context_window, 1000);
        assert_eq!(limits.input_limit, Some(900));
    }

    #[test]
    fn model_descriptor_ignores_unknown_router_fields() {
        let m: Model = serde_json::from_value(json!({
            "id": "claude-sonnet-4", "provider": "anthropic",
            "context_window": 200_000, "max_output_tokens": 16_000,
            "thinking_budgets": { "low": 1024, "high": 8192 },
            "display_name": "Sonnet", "supports_tools": true,
            "pricing": { "input": 3.0 }
        }))
        .unwrap();
        assert_eq!(m.context_window, 200_000);
        assert_eq!(
            m.thinking_budgets.unwrap().get(&ThinkingLevel::High),
            Some(&8192)
        );
    }

    #[test]
    fn thinking_level_wire_names_are_snake_case() {
        for (level, wire) in [
            (ThinkingLevel::Minimal, "minimal"),
            (ThinkingLevel::Low, "low"),
            (ThinkingLevel::Medium, "medium"),
            (ThinkingLevel::High, "high"),
            (ThinkingLevel::Xhigh, "xhigh"),
        ] {
            assert_eq!(serde_json::to_value(level).unwrap(), json!(wire));
        }
    }

    #[test]
    fn has_function_result_block_detects_inline_results() {
        let user_with_result: AgentMessage = serde_json::from_value(json!({
            "role": "user",
            "content": [{ "type": "function_result", "function_call_id": "c1", "content": [] }],
            "timestamp": 1
        }))
        .unwrap();
        assert!(user_with_result.has_function_result_block());

        let plain: AgentMessage = serde_json::from_value(json!({
            "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1
        }))
        .unwrap();
        assert!(!plain.has_function_result_block());
    }
}
