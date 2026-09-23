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
    ProviderAbortRequest, ProviderAbortResponse, ProviderReadyAck, ProviderStreamInput,
    ProviderStreamOutput, RefreshModelsRequest, RefreshModelsResponse, RouterReadyEvent,
};

pub const STREAM_ID: &str = "provider::openai::stream";
pub const STREAM_DESC: &str =
    "Stream an OpenAI response: resolve credentials, call the configured \
     Responses or Chat Completions endpoint, and relay AssistantMessageEvent frames to writer_ref.";

pub const ABORT_ID: &str = "provider::openai::abort";
pub const ABORT_DESC: &str = "Cancel the in-flight upstream stream for a request_id \
     (router::abort fan-out), stopping billed generation immediately.";

pub const REFRESH_MODELS_ID: &str = "provider::openai::refresh_models";
pub const REFRESH_MODELS_DESC: &str = "Refresh the OpenAI catalog slice from GET /v1/models and \
     reconcile it through the router; returns the model count written.";

pub const ON_ROUTER_READY_ID: &str = "provider::openai::on_router_ready";
pub const ON_ROUTER_READY_DESC: &str =
    "Internal: router::ready subscriber that re-declares this provider and refreshes its catalog.";

pub const EMBED_ID: &str = "provider::openai::embed";
pub const EMBED_DESC: &str =
    "Batch text embeddings via the OpenAI embeddings endpoint, using the router-resolved \
     credential. One vector per input, order preserved. Default model text-embedding-3-small.";

pub const IMAGE_GENERATE_ID: &str = "provider::openai::image::generate";
pub const IMAGE_GENERATE_DESC: &str =
    "Generate one image from a text prompt with gpt-image-2.5-sunburst or gpt-image-2.5-flare \
     (OpenAI Images API, router-resolved credential). Returns viewable content blocks \
     (image + caption) that render inline, and saves the file under data/provider-openai/images/ \
     (details.path) for other processes to pick up.";

pub const IMAGE_READ_ID: &str = "provider::openai::image::read";
pub const IMAGE_READ_DESC: &str =
    "Read back an image saved by image::generate (its details.path or bare file name under \
     data/provider-openai/images/) as viewable content blocks: a JPEG preview (default, fits the \
     harness result cap) or the full bytes. Refuses any path outside that folder.";

pub const COUNT_TOKENS_ID: &str = "provider::openai::count_tokens";
pub const COUNT_TOKENS_DESC: &str =
    "Count prompt tokens for {model, system_prompt?, tools?, messages} locally with the \
     tiktoken tokenizers; never runs the model and costs nothing.";

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
        spec::<ProviderAbortRequest, ProviderAbortResponse>(ABORT_ID, ABORT_DESC),
        spec::<RefreshModelsRequest, RefreshModelsResponse>(REFRESH_MODELS_ID, REFRESH_MODELS_DESC),
        spec::<RouterReadyEvent, ProviderReadyAck>(ON_ROUTER_READY_ID, ON_ROUTER_READY_DESC),
        spec::<crate::embed::EmbedRequest, crate::embed::EmbedResponse>(EMBED_ID, EMBED_DESC),
        spec::<crate::image::ImageGenerateRequest, crate::image::ImageGenerateResponse>(
            IMAGE_GENERATE_ID,
            IMAGE_GENERATE_DESC,
        ),
        spec::<crate::image::ImageReadRequest, crate::image::ImageReadResponse>(
            IMAGE_READ_ID,
            IMAGE_READ_DESC,
        ),
        spec::<crate::count_tokens::CountTokensRequest, crate::count_tokens::CountTokensResponse>(
            COUNT_TOKENS_ID,
            COUNT_TOKENS_DESC,
        ),
    ]
}
