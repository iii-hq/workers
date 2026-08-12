//! Wire-surface catalog for the `router::*` functions — the single source of
//! truth for each function's id, registration description, and
//! schemars-derived request/response schemas.
//!
//! Golden-tested in `tests/schemas.rs`; keep in lockstep with
//! [`crate::register::register_router`]. Schema generation MUST mirror
//! iii-sdk's internal `json_schema_for` (`SchemaSettings::draft07()` on the
//! handler's request/response types): `RegisterFunction::new_async` /
//! `new_async_with_bad_request` auto-extract schemas from the SAME structs
//! referenced here, with the same schemars 0.8 generator settings, so a catalog
//! snapshot pins exactly what registration emits.

use crate::chat::chat::{ChatCall, ChatFnInput};
use crate::types::router::{
    AbortRequest, AbortResponse, ChatResponse, CompleteResponse, ConfigChangedEvent,
    ModelBudgetRequest, ModelBudgetResponse, ModelGetRequest, ModelGetResponse, ModelsListRequest,
    ModelsListResponse, ModelsReconcileRequest, ModelsReconcileResponse, ModelsSupportsRequest,
    ModelsSupportsResponse, ProviderListRequest, ProviderListResponse, ProviderRegisterRequest,
    ProviderRegisterResponse, ProviderResolveRequest, ProviderResolveResponse, RouteRequest,
    RouteResponse, RouterAck, SystemPromptGetRequest, SystemPromptGetResponse,
    UpdateCredentialRequest, UpdateCredentialResponse,
};

// ── function id + description constants — consumed by both register_router and
//    catalog() so the registered surface and the golden stay in lockstep. ──────

pub const CHAT_ID: &str = "router::chat";
pub const CHAT_DESC: &str =
    "Stream a chat completion: route {model, provider?} to a provider, relay \
     assistant frames to writer_ref, and return the terminal response.";

pub const COMPLETE_ID: &str = "router::complete";
pub const COMPLETE_DESC: &str = "Non-streaming convenience over router::chat: run the turn on an \
     internal channel and return the final assistant message + usage.";

pub const ABORT_ID: &str = "router::abort";
pub const ABORT_DESC: &str =
    "Abort an in-flight router::chat/complete by request_id; reports whether a live request was cancelled.";

pub const EMBED_ID: &str = "router::embed";
pub const EMBED_DESC: &str =
    "Batch text embeddings through a provider's provider::<id>::embed surface. Names a provider \
     or discovers the first embed-capable one from the live registry; one vector per input, \
     order preserved.";

pub const COUNT_TOKENS_ID: &str = "router::count_tokens";
pub const COUNT_TOKENS_DESC: &str =
    "Count prompt tokens for {model, provider?, system_prompt?, tools?, messages} through the \
     resolved provider's provider::<id>::count_tokens surface; never runs the model and costs \
     nothing.";

pub const MODELS_LIST_ID: &str = "router::models::list";
pub const MODELS_LIST_DESC: &str =
    "List catalog models, optionally filtered by provider and/or a capability flag.";

pub const MODELS_GET_ID: &str = "router::models::get";
pub const MODELS_GET_DESC: &str =
    "Read one catalog model by {provider, id}; null when the model is not registered.";

pub const MODELS_BUDGET_ID: &str = "router::models::budget";
pub const MODELS_BUDGET_DESC: &str =
    "Resolve the effective model output budget using the same precedence as router::chat.";

pub const MODELS_SUPPORTS_ID: &str = "router::models::supports";
pub const MODELS_SUPPORTS_DESC: &str =
    "Check whether a model supports a capability flag (fails open for unknown models).";

pub const PROVIDER_LIST_ID: &str = "router::provider::list";
pub const PROVIDER_LIST_DESC: &str =
    "List registered providers with their configured/available status.";

pub const SYSTEM_PROMPT_GET_ID: &str = "router::system_prompt::get";
pub const SYSTEM_PROMPT_GET_DESC: &str =
    "Effective identity prompt for {provider?} (default_provider when omitted): \
     operator override when set, else provider-declared; null when absent.";

pub const ROUTE_ID: &str = "router::route";
pub const ROUTE_DESC: &str = "Read-only routing preview: resolve {model, provider?} to the chosen \
     provider + ordered candidates without streaming.";

pub const PROVIDER_REGISTER_ID: &str = "router::provider::register";
pub const PROVIDER_REGISTER_DESC: &str = "Provider self-declaration at attach time (token-gated \
     upsert); composes the configuration entry schema and reconciles static models.";

pub const PROVIDER_RESOLVE_ID: &str = "router::provider::resolve";
pub const PROVIDER_RESOLVE_DESC: &str =
    "Resolve a provider's effective credential + api_url + max_tokens (token-gated).";

pub const UPDATE_CREDENTIAL_ID: &str = "router::provider::update_credential";
pub const UPDATE_CREDENTIAL_DESC: &str = "OAuth write-back: store a provider credential in the \
     configuration entry under the entry write lock (token-gated).";

pub const MODELS_RECONCILE_ID: &str = "router::models::reconcile";
pub const MODELS_RECONCILE_DESC: &str =
    "Replace a provider's catalog slice — the only catalog write path (token-gated).";

pub const ON_FUNCTIONS_CHANGED_ID: &str = "router::on_functions_changed";
pub const ON_FUNCTIONS_CHANGED_DESC: &str =
    "Internal: a worker's function registrations changed — re-discover live \
     providers and nudge them to re-declare, so a provider that reconnected \
     is resolvable again without waiting for its own catalog timer.";

pub const ON_CONFIG_CHANGED_ID: &str = "router::on_config_changed";
pub const ON_CONFIG_CHANGED_DESC: &str =
    "Internal: reactively reload the in-memory configuration snapshot and \
     fan out provider model discovery after configuration changes.";

/// One function's complete agent-facing wire surface: id, registration
/// description, and the schemars-derived request/response schemas.
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

fn schema_of<T: schemars::JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req, Resp>(function_id: &'static str, description: &'static str) -> FunctionSpec
where
    Req: schemars::JsonSchema,
    Resp: schemars::JsonSchema,
{
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// The full wire-surface catalog, in registration order. Golden-tested in
/// `tests/schemas.rs`; keep in lockstep with `register::register_router`.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<ChatFnInput, ChatResponse>(CHAT_ID, CHAT_DESC),
        spec::<ChatCall, CompleteResponse>(COMPLETE_ID, COMPLETE_DESC),
        spec::<AbortRequest, AbortResponse>(ABORT_ID, ABORT_DESC),
        spec::<crate::embed::RouterEmbedRequest, crate::embed::RouterEmbedResponse>(
            EMBED_ID, EMBED_DESC,
        ),
        spec::<
            crate::count_tokens::RouterCountTokensRequest,
            crate::count_tokens::RouterCountTokensResponse,
        >(COUNT_TOKENS_ID, COUNT_TOKENS_DESC),
        spec::<ModelsListRequest, ModelsListResponse>(MODELS_LIST_ID, MODELS_LIST_DESC),
        spec::<ModelGetRequest, Option<ModelGetResponse>>(MODELS_GET_ID, MODELS_GET_DESC),
        spec::<ModelBudgetRequest, Option<ModelBudgetResponse>>(
            MODELS_BUDGET_ID,
            MODELS_BUDGET_DESC,
        ),
        spec::<ModelsSupportsRequest, ModelsSupportsResponse>(
            MODELS_SUPPORTS_ID,
            MODELS_SUPPORTS_DESC,
        ),
        spec::<ProviderListRequest, ProviderListResponse>(PROVIDER_LIST_ID, PROVIDER_LIST_DESC),
        spec::<SystemPromptGetRequest, SystemPromptGetResponse>(
            SYSTEM_PROMPT_GET_ID,
            SYSTEM_PROMPT_GET_DESC,
        ),
        spec::<RouteRequest, RouteResponse>(ROUTE_ID, ROUTE_DESC),
        spec::<ProviderRegisterRequest, ProviderRegisterResponse>(
            PROVIDER_REGISTER_ID,
            PROVIDER_REGISTER_DESC,
        ),
        spec::<ProviderResolveRequest, ProviderResolveResponse>(
            PROVIDER_RESOLVE_ID,
            PROVIDER_RESOLVE_DESC,
        ),
        spec::<UpdateCredentialRequest, UpdateCredentialResponse>(
            UPDATE_CREDENTIAL_ID,
            UPDATE_CREDENTIAL_DESC,
        ),
        spec::<ModelsReconcileRequest, ModelsReconcileResponse>(
            MODELS_RECONCILE_ID,
            MODELS_RECONCILE_DESC,
        ),
        spec::<ConfigChangedEvent, RouterAck>(ON_CONFIG_CHANGED_ID, ON_CONFIG_CHANGED_DESC),
    ]
}
