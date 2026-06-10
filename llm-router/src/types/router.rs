use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::types::channel::StreamChannelRef;
use crate::types::credential::Credential;
use crate::types::events::{StopReason, Usage};
use crate::types::messages::{AgentMessage, AssistantMessage};
use crate::types::model::{AgentFunction, Model, ThinkingLevel};

// ── consumer surface ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseFormat {
    pub r#type: String, // "json"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
}

/// Input of the `router::chat` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub writer_ref: StreamChannelRef, // direction "write"; the caller's channel
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>, // router::abort correlation; generated when omitted
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    pub messages: Vec<AgentMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AgentFunction>>, // adapter boundary (pending rename → functions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<BTreeMap<String, Value>>, // namespaced by provider id
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorShape {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatResponse {
    pub ok: bool,
    pub provider: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorShape>,
}

/// Output of the `router::complete` iii function (non-streaming convenience).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompleteResponse {
    pub message: AssistantMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    pub provider: String,
    pub model: String,
}

/// Input of the `router::abort` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbortRequest {
    pub request_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbortResponse {
    pub aborted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: String,
    pub display_name: String,
    pub configured: bool,
    pub available: bool,
    pub supports_model_listing: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderListResponse {
    pub providers: Vec<ProviderInfo>,
}

// ── provider protocol ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderDefaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Input of the `router::provider::register` iii function — a provider
/// worker's self-declaration at attach time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderDeclaration {
    pub id: String, // also the provider::<id>::* prefix and config key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_env_var: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defaults: Option<ProviderDefaults>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_model_listing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<Model>>, // static catalog slice; reconciled at registration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>, // self-reported; availability mapping only, never authorization
}

/// registration_token: spec adaptation — the engine exposes no caller identity,
/// so identity binding is a bearer token; only its sha256 hash is persisted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderRegisterResponse {
    pub ok: bool,
    pub id: String,
    pub registration_token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialSource {
    Config,
    Env,
    None,
}

/// Output of the `router::provider::resolve` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderResolveResponse {
    pub configured: bool,
    pub source: CredentialSource,
    pub credential: Option<Credential>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
}

/// Output of the `router::models::reconcile` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelsReconcileResponse {
    pub provider: String,
    pub count: usize,
}

/// Input of a provider worker's `provider::<id>::stream` iii function —
/// what the router forwards per attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderStreamInput {
    pub writer_ref: StreamChannelRef, // direction "write" (router-owned in relay mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    pub model: String,
    pub messages: Vec<AgentMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AgentFunction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>, // effective budget, resolved + clamped by the router
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>, // this provider's slice, verbatim
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_meta: Option<Model>, // hint, never source of truth
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_key: Option<String>,
}

// ── event payloads ──────────────────────────────────────────────────────────

/// Payload published on the `router::models::changed` pubsub topic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelsChangedPayload {
    pub provider: String,
    pub count: usize,
}

/// Payload published on the `router::provider::changed` pubsub topic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderChangedPayload {
    pub provider: String,
    pub op: String, // "register" | "available" | "unavailable"
}
