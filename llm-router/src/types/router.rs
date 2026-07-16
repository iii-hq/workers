use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::types::credential::Credential;
use crate::types::events::{StopReason, Usage};
use crate::types::messages::{AgentMessage, AssistantMessage};
use crate::types::model::{AgentFunction, Model, ThinkingLevel};
use iii_sdk::channel::StreamChannelRef;

// ── consumer surface ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ResponseFormat {
    pub r#type: String, // "json"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
}

/// Input of the `router::chat` iii function.
/// (No `PartialEq`: `iii_sdk::StreamChannelRef` doesn't implement it.)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorShape {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompleteResponse {
    pub message: AssistantMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    pub provider: String,
    pub model: String,
}

/// Input of the `router::abort` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AbortRequest {
    pub request_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AbortResponse {
    pub aborted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderInfo {
    pub id: String,
    pub display_name: String,
    pub configured: bool,
    pub available: bool,
    pub supports_model_listing: bool,
}
/// Input of `router::provider::list` — takes no arguments. A struct (rather
/// than `Value`) keeps the request schema concrete; unknown fields (e.g. the
/// engine-injected `_caller_worker_id`) are ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderListRequest {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderListResponse {
    pub providers: Vec<ProviderInfo>,
}

// ── provider protocol ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    pub system_prompt: Option<String>, // provider-authored identity prompt, served via router::system_prompt::get
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>, // self-reported; availability mapping only, never authorization
}

/// Input of the `router::system_prompt::get` iii function. Unknown fields
/// (e.g. the engine-injected `_caller_worker_id`) are ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SystemPromptGetRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>, // absent → the configured default_provider
}

/// Output of `router::system_prompt::get`: the effective identity prompt for
/// the resolved provider — operator override (config slice `system_prompt`,
/// when set) → provider-declared → null. Null also when the provider is
/// unknown or no provider resolves; callers treat null as "use your own
/// default prompt".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SystemPromptGetResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

/// registration_token: spec adaptation — the engine exposes no caller identity,
/// so identity binding is a bearer token; only its sha256 hash is persisted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderRegisterResponse {
    pub ok: bool,
    pub id: String,
    pub registration_token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CredentialSource {
    Config,
    Env,
    None,
}

/// Output of the `router::provider::resolve` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsReconcileResponse {
    pub provider: String,
    pub count: usize,
}

/// Input of a provider worker's `provider::<id>::stream` iii function —
/// what the router forwards per attempt.
/// (No `PartialEq`: `iii_sdk::StreamChannelRef` doesn't implement it.)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

/// Output of a provider's `provider::<id>::stream` (spec § stream contract):
/// the function streams frames to `writer_ref` and returns this ack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderStreamOutput {
    pub ok: bool,
}

/// Input of a provider's `provider::<id>::refresh_models` — takes no arguments.
/// A struct (not `Value`) keeps the request schema concrete; unknown fields
/// (e.g. the engine-injected `_caller_worker_id`) are ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RefreshModelsRequest {}

/// Output of `provider::<id>::refresh_models`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RefreshModelsResponse {
    pub ok: bool,
    pub count: usize,
}

/// Event delivered to a provider's `provider::<id>::on_router_ready` (the
/// `router::ready` trigger payload, currently `{}`). Unknown fields are ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RouterReadyEvent {}

/// Ack returned by a provider's `provider::<id>::on_router_ready`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderReadyAck {
    pub ok: bool,
}

// ── event payloads ──────────────────────────────────────────────────────────

/// Payload emitted on the `router::models::changed` trigger type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsChangedPayload {
    pub provider: String,
    pub count: usize,
}

/// Payload emitted on the `router::provider::changed` trigger type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderChangedPayload {
    pub provider: String,
    pub op: String, // "register" | "available" | "unavailable"
}

// ── typed function request/response wire types ───────────────────────────────
//
// These mirror the ad-hoc JSON each handler read/wrote before; promoting them
// to structs (with `JsonSchema`) is what lets the SDK emit real request/response
// schemas instead of the permissive `AnyValue` schema a `Value` handler yields.

/// Input of `router::models::list`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsListRequest {
    /// Filter to a single provider id (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Keep only models that support this capability flag (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
}

/// Output of `router::models::list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsListResponse {
    pub models: Vec<Model>,
}

/// Input of `router::models::get`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelGetRequest {
    /// Provider id that owns the model.
    #[serde(default)]
    pub provider: String,
    /// Model id to look up.
    #[serde(default)]
    pub id: String,
}

/// Output of `router::models::get` (the function returns `null` when the model
/// is not registered — the cold-window signal).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelGetResponse {
    pub model: Model,
}

/// Input of `router::models::budget`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelBudgetRequest {
    /// Provider id that owns the model. Empty resolves an unambiguous model id.
    #[serde(default)]
    pub provider: String,
    /// Model id to budget.
    #[serde(default)]
    pub id: String,
    /// Optional caller-requested output budget. When absent, the same provider
    /// and router defaults used by `router::chat` apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
}

/// Effective limits `router::chat` will use for this model and request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelBudgetResponse {
    pub model: Model,
    pub effective_max_output_tokens: u64,
}

/// Input of `router::models::supports`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsSupportsRequest {
    /// Provider id that owns the model.
    #[serde(default)]
    pub provider: String,
    /// Model id to check.
    #[serde(default)]
    pub id: String,
    /// Capability flag to check (e.g. `structured_output`, `vision`).
    #[serde(default)]
    pub capability: String,
}

/// Output of `router::models::supports`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsSupportsResponse {
    pub supported: bool,
}

/// Input of `router::route` — the read-only routing preview.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RouteRequest {
    /// Model id to route.
    #[serde(default)]
    pub model: String,
    /// Pin an explicit provider, bypassing heuristics (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// Output of `router::route`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RouteResponse {
    pub provider: String,
    pub candidates: Vec<String>,
}

/// Input of `router::provider::register` — a provider worker's declaration plus
/// the optional re-registration token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderRegisterRequest {
    #[serde(flatten)]
    pub declaration: ProviderDeclaration,
    /// Registration token proving ownership on re-register (omit on first declare).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Input of `router::provider::resolve`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderResolveRequest {
    /// Provider id to resolve credentials/config for.
    #[serde(default)]
    pub id: String,
    /// Registration token gating the resolve (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Input of `router::provider::update_credential` (OAuth write-back).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateCredentialRequest {
    /// Provider id whose credential slice is being written.
    #[serde(default)]
    pub id: String,
    /// Registration token gating the write (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The credential object to store (provider-specific shape).
    #[serde(default)]
    pub credential: Value,
}

/// Output of `router::provider::update_credential`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateCredentialResponse {
    pub ok: bool,
}

/// Input of `router::models::reconcile` — the only catalog write path.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsReconcileRequest {
    /// Provider whose catalog slice is being replaced.
    #[serde(default)]
    pub provider: String,
    /// Registration token gating the write (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The full replacement set of models for this provider.
    #[serde(default)]
    pub models: Vec<Model>,
}

/// Advisory configuration-change event delivered to
/// `router::on_config_changed`. The handler ignores event values and re-fetches
/// the authoritative entry before replacing its in-memory snapshot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigChangedEvent {
    /// Configuration id that changed (advisory).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Generic acknowledgement returned by trigger-bound handlers whose result is
/// not consumed by callers (kept typed so the response schema is concrete).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RouterAck {
    pub ok: bool,
}
