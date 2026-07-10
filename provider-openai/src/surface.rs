//! Wire-surface catalog for the `provider::openai::*` functions — the single
//! source of truth for each function's id, registration description, and
//! schemars-derived request/response schemas.
//!
//! Golden-tested in `tests/schemas.rs`; keep in lockstep with
//! [`crate::register::register_provider`]. Schema generation MUST mirror
//! iii-sdk's internal `json_schema_for` (`SchemaSettings::draft07()` on the
//! handler's request/response types) so a catalog snapshot pins exactly what
//! registration emits.

use llm_router::types::router::{
    ProviderReadyAck, ProviderStreamInput, ProviderStreamOutput, RefreshModelsRequest,
    RefreshModelsResponse, RouterReadyEvent,
};

pub const STREAM_ID: &str = "provider::openai::stream";
pub const STREAM_DESC: &str =
    "Stream an OpenAI response: resolve credentials, call the configured \
     Responses or Chat Completions endpoint, and relay AssistantMessageEvent frames to writer_ref.";

pub const REFRESH_MODELS_ID: &str = "provider::openai::refresh_models";
pub const REFRESH_MODELS_DESC: &str = "Refresh the OpenAI catalog slice from GET /v1/models and \
     reconcile it through the router; returns the model count written.";

pub const ON_ROUTER_READY_ID: &str = "provider::openai::on_router_ready";
pub const ON_ROUTER_READY_DESC: &str =
    "Internal: router::ready subscriber that re-declares this provider and refreshes its catalog.";

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
/// `tests/schemas.rs`; keep in lockstep with `register::register_provider`.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<ProviderStreamInput, ProviderStreamOutput>(STREAM_ID, STREAM_DESC),
        spec::<RefreshModelsRequest, RefreshModelsResponse>(REFRESH_MODELS_ID, REFRESH_MODELS_DESC),
        spec::<RouterReadyEvent, ProviderReadyAck>(ON_ROUTER_READY_ID, ON_ROUTER_READY_DESC),
    ]
}
