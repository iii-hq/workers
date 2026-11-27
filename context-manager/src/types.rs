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

use crate::core::estimate::{ByRole, EstimatorKind};

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
        /// Optional link to the stored original in the session-manager
        /// attachment store. Transcript bookkeeping for lazy readers; the
        /// context manager neither reads nor strips it — it only has to ride
        /// through compaction untouched, and stay absent on the wire when the
        /// block never carried one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attachment_id: Option<String>,
    },
    /// Reference to an attachment kept in the session-manager's attachment
    /// store. Carries no bytes and no model-visible text; the harness strips
    /// these before assembling context, so here it only has to deserialize
    /// (a transcript with attachments must never fail `count-tokens`). It
    /// contributes nothing to estimates, pruning or summaries.
    File {
        attachment_id: String,
        name: String,
        mime: String,
        size: u64,
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

    pub(crate) fn content_mut(&mut self) -> &mut Vec<ContentBlock> {
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
    /// The harness asks the provider not to reason; reserves no budget
    /// unless the model declares one for it.
    Off,
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
    /// Whether the model accepts image content blocks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
}

/// An invocation-surface schema entry (README.md § AgentFunction) —
/// accepted by `context::count-tokens` so callers can include the
/// `agent_call` schema in their estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentFunction {
    /// Invocation surface name (`agent_call`, `submit_result`, or a
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

/// Per-role token breakdown of the `messages` array.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ByRoleTokens {
    pub user: u64,
    pub assistant: u64,
    pub function_result: u64,
    pub custom: u64,
}

impl From<ByRole> for ByRoleTokens {
    fn from(by_role: ByRole) -> Self {
        ByRoleTokens {
            user: by_role.user,
            assistant: by_role.assistant,
            function_result: by_role.function_result,
            custom: by_role.custom,
        }
    }
}

/// Which estimator produced the count.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EstimatorName {
    Tokenizer,
    Heuristic,
}

impl From<EstimatorKind> for EstimatorName {
    fn from(kind: EstimatorKind) -> Self {
        match kind {
            EstimatorKind::Tokenizer => EstimatorName::Tokenizer,
            EstimatorKind::Heuristic => EstimatorName::Heuristic,
        }
    }
}

/// `router::chat`'s `provider_options`, namespaced by provider id (e.g.
/// `{"claude-code": {"account": "…"}}`). Forwarded verbatim; the router
/// hands each provider its own slice.
pub type ProviderOptions = BTreeMap<String, Value>;

/// Why a summariser produced no summary, in words a caller can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The account's usage limit is reached (llm-router's
    /// `error_details.usage_limit`): waiting a moment does not help.
    UsageLimit,
    /// The provider is busy right now (`rate_limited` without a usage limit).
    RateLimited,
    /// The input did not fit: the provider refused it as larger than the
    /// model's window, or the history needs more summary calls, or more room
    /// than the window leaves. The only kind a larger window would fix.
    InputTooLarge,
    /// The summary took longer than its time budget, or the router's stream
    /// timed out.
    Timeout,
    /// No summariser: llm-router is absent, or no provider can serve it.
    Unavailable,
    /// Anything else: an authentication problem, a transient provider error,
    /// an empty summary.
    Other,
}

/// The usage window an account's limit belongs to (llm-router's
/// `UsageWindow`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsageWindow {
    FiveHour,
    Week,
}

/// llm-router's `error_details` (D147), passed through from the router's
/// error: what an error's kind alone does not say.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorDetails {
    /// The account's usage limit is reached.
    #[serde(default)]
    pub usage_limit: bool,
    /// Which limit was reached, when the provider said so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<UsageWindow>,
    /// When that limit resets, in milliseconds since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<u64>,
}

impl ErrorDetails {
    /// Read `error_details` field by field, as llm-router does: an object,
    /// else nothing; a `window` or `resets_at_ms` this version cannot read is
    /// absent and never costs `usage_limit`.
    pub fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        Some(Self {
            usage_limit: object
                .get("usage_limit")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            window: object
                .get("window")
                .and_then(|window| serde_json::from_value(window.clone()).ok()),
            resets_at_ms: object.get("resets_at_ms").and_then(Value::as_u64),
        })
    }
}

/// The cause of a failed summary: `context::compact`'s `overflow`, the
/// assemble's `compaction.failure`, and the tail of a `context/overflow`
/// error that follows a failed inline compaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Failure {
    pub kind: FailureKind,
    /// What happened, in a sentence: the router's user-facing message when
    /// the router said why, else this worker's own.
    pub message: String,
    /// The router's `error_details`, when it sent them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_details: Option<ErrorDetails>,
}

impl Failure {
    pub fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            error_details: None,
        }
    }

    pub fn with_details(mut self, details: Option<ErrorDetails>) -> Self {
        self.error_details = details;
        self
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// A failure with no named cause (`other`).
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::new(FailureKind::Other, message)
    }
}

/// A failure with no named cause (`other`).
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::new(FailureKind::Other, message)
    }
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
        // A legacy image (no link) never gains an `attachment_id` key.
        assert_eq!(
            round[1],
            json!({ "type": "image", "mime": "image/png", "data": "aGk=" })
        );
    }

    #[test]
    fn image_attachment_id_round_trips_untouched_and_defaults_to_none() {
        let linked = json!({
            "type": "image",
            "mime": "image/png",
            "data": "aGk=",
            "attachment_id": "a_9c1d"
        });
        let block: ContentBlock = serde_json::from_value(linked.clone()).unwrap();
        assert_eq!(
            block,
            ContentBlock::Image {
                mime: "image/png".into(),
                data: "aGk=".into(),
                attachment_id: Some("a_9c1d".into()),
            }
        );
        assert_eq!(serde_json::to_value(&block).unwrap(), linked);

        let legacy: ContentBlock =
            serde_json::from_value(json!({ "type": "image", "mime": "image/png", "data": "aGk=" }))
                .unwrap();
        assert!(matches!(
            legacy,
            ContentBlock::Image {
                attachment_id: None,
                ..
            }
        ));

        // Inside a user message the link survives a message-level round trip.
        let msg: AgentMessage = serde_json::from_value(json!({
            "role": "user",
            "content": [{ "type": "text", "text": "look" }, linked],
            "timestamp": 1
        }))
        .unwrap();
        let round = serde_json::to_value(&msg).unwrap();
        assert_eq!(round["content"][1]["attachment_id"], "a_9c1d");
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
            "supports_vision": false,
            "display_name": "Sonnet", "supports_tools": true,
            "pricing": { "input": 3.0 }
        }))
        .unwrap();
        assert_eq!(m.context_window, 200_000);
        assert_eq!(
            m.thinking_budgets.unwrap().get(&ThinkingLevel::High),
            Some(&8192)
        );
        assert_eq!(m.supports_vision, Some(false));
    }

    #[test]
    fn thinking_level_wire_names_are_snake_case() {
        for (level, wire) in [
            (ThinkingLevel::Off, "off"),
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
