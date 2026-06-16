use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::types::credential::Credential;
use crate::types::events::{StopReason, Usage};
use crate::types::messages::{AgentMessage, AssistantMessage};
use crate::types::model::{AgentFunction, Model, ThinkingLevel};
use iii_sdk::StreamChannelRef;

// ── consumer surface ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ResponseFormat {
    pub r#type: String, // "json"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
}

/// Input of the `router::chat` iii function.
/// (No `PartialEq`: `iii_sdk::StreamChannelRef` doesn't implement it.)
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ErrorShape {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CompleteResponse {
    pub message: AssistantMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    pub provider: String,
    pub model: String,
}

/// Input of the `router::abort` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AbortRequest {
    pub request_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AbortResponse {
    pub aborted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderInfo {
    pub id: String,
    pub display_name: String,
    pub configured: bool,
    pub available: bool,
    pub supports_model_listing: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderListResponse {
    pub providers: Vec<ProviderInfo>,
}

// ── provider protocol ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderRegisterResponse {
    pub ok: bool,
    pub id: String,
    pub registration_token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CredentialSource {
    Config,
    Env,
    None,
}

/// Output of the `router::provider::resolve` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelsReconcileResponse {
    pub provider: String,
    pub count: usize,
}

/// Input of a provider worker's `provider::<id>::stream` iii function —
/// what the router forwards per attempt.
/// (No `PartialEq`: `iii_sdk::StreamChannelRef` doesn't implement it.)
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelsChangedPayload {
    pub provider: String,
    pub count: usize,
}

/// Payload published on the `router::provider::changed` pubsub topic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderChangedPayload {
    pub provider: String,
    pub op: String, // "register" | "available" | "unavailable"
}

// ── published-schema request/response shapes ─────────────────────────────────
//
// These describe the wire surface for `router::*` functions whose handlers
// stay on `serde_json::Value` (tolerant parsing, streaming sinks, bare-null
// answers — see `wire_schema`). They are schema-only: the handlers do not
// deserialize into them, so the runtime contract is unchanged. Keeping them
// here, next to the response types, makes the published API reference precise
// instead of "unknown".

/// Input of the `router::complete` iii function — the chat input without the
/// streaming `writer_ref` (complete owns an internal channel). Mirrors
/// `ChatRequest` field-for-field minus `writer_ref`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CompleteRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    pub messages: Vec<AgentMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AgentFunction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<BTreeMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// Input of the `router::route` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RouteRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}
/// Output of the `router::route` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RouteResponse {
    pub provider: String,
    pub candidates: Vec<String>,
}

/// Input of the `router::models::list` iii function (both filters optional).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelsListRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
}
/// Output of the `router::models::list` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelsListResponse {
    pub models: Vec<Model>,
}

/// Input of the `router::models::get` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelsGetRequest {
    pub provider: String,
    pub id: String,
}
/// Success envelope of `router::models::get`. The function answers with this
/// object when the model is registered, or a bare `null` when it is not (the
/// cold-window signal) — see `wire_schema` for the published `anyOf`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelsGetResponse {
    pub model: Model,
}

/// Input of the `router::models::supports` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelsSupportsRequest {
    pub provider: String,
    pub id: String,
    pub capability: String,
}
/// Output of the `router::models::supports` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelsSupportsResponse {
    pub supported: bool,
}

/// Input of the `router::provider::list` iii function — takes no parameters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderListRequest {}

/// Input of the `router::provider::resolve` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderResolveRequest {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>, // registration bearer token
}

/// Input of the `router::provider::update_credential` iii function. The
/// `credential` stays an opaque object (validated `is_object()` by the
/// handler) so the existing `router/invalid_request` message is preserved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UpdateCredentialRequest {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    // Runtime stays `Value` (the handler forwards it verbatim after an
    // `is_object()` gate); the published schema describes that object contract
    // rather than the permissive any-value `Value` renders as.
    #[schemars(with = "BTreeMap<String, Value>")]
    pub credential: Value,
}

/// Input of the `router::models::reconcile` iii function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelsReconcileRequest {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    pub models: Vec<Model>,
}

/// A bare `{ "ok": true }` acknowledgement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OkResponse {
    pub ok: bool,
}

/// Engine topology event delivered to `router::on_worker_available` on the
/// `engine::workers-available` subscribe trigger. Permissive on purpose —
/// the handler tolerates several shapes and ignores the rest.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WorkerAvailableEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
}

/// Engine configuration-change event delivered to `router::on_config_changed`
/// on the `configuration:updated` trigger for the `llm-router` entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConfigChangedEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_value: Option<Value>, // the full updated operator config entry
}

// ── provider protocol acks (returned by provider workers) ────────────────────

/// Synchronous acknowledgement returned by `provider::<id>::stream` and
/// `provider::<id>::on_router_ready` — the real product of `stream` is the
/// frames written to the caller's channel, not this value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderAck {
    pub ok: bool,
}

/// Acknowledgement returned by `provider::<id>::refresh_models`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RefreshModelsAck {
    pub ok: bool,
    pub count: usize,
}

/// A function that takes no caller parameters (e.g. `refresh_models`, or a
/// re-declare handler fired by an engine pubsub event it ignores). Published
/// as an empty object schema rather than the permissive `AnyValue`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NoParams {}
