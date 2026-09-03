//! Wire-surface catalog for the `provider::sarvam::*` functions — the single
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

pub const STREAM_ID: &str = "provider::sarvam::stream";
pub const STREAM_DESC: &str = "Stream a Sarvam chat completion: resolve credentials, call the \
     upstream Chat Completions API, and relay AssistantMessageEvent frames to writer_ref.";

pub const ABORT_ID: &str = "provider::sarvam::abort";
pub const ABORT_DESC: &str = "Cancel the in-flight upstream stream for a request_id \
     (router::abort fan-out), stopping billed generation immediately.";

pub const REFRESH_MODELS_ID: &str = "provider::sarvam::refresh_models";
pub const REFRESH_MODELS_DESC: &str = "Reconcile the curated Sarvam catalog slice, chat and speech \
     models, through the router (Sarvam has no models-listing endpoint); returns the count written.";

pub const ON_ROUTER_READY_ID: &str = "provider::sarvam::on_router_ready";
pub const ON_ROUTER_READY_DESC: &str =
    "Internal: router::ready subscriber that re-declares this provider and refreshes its catalog.";

pub const COUNT_TOKENS_ID: &str = "provider::sarvam::count_tokens";
pub const COUNT_TOKENS_DESC: &str =
    "Estimate prompt tokens for {model, system_prompt?, tools?, messages} locally with tiktoken; \
     never runs the model and costs nothing.";

pub const TRANSCRIBE_ID: &str = "provider::sarvam::transcribe";
pub const TRANSCRIBE_DESC: &str =
    "Speech to text on Sarvam Saaras behind router::transcribe: base64 audio in, text with timed \
     segments and the detected language out; 22 Indian languages and English.";

pub const SPEAK_ID: &str = "provider::sarvam::speak";
pub const SPEAK_DESC: &str =
    "Text to speech on Sarvam Bulbul behind router::speak: text, a speaker name and a language in, \
     base64 audio out (mp3, wav, pcm16, opus).";

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
        spec::<crate::count_tokens::CountTokensRequest, crate::count_tokens::CountTokensResponse>(
            COUNT_TOKENS_ID,
            COUNT_TOKENS_DESC,
        ),
        spec::<crate::speech::TranscribeRequest, crate::speech::TranscribeResponse>(
            TRANSCRIBE_ID,
            TRANSCRIBE_DESC,
        ),
        spec::<crate::speech::SpeakRequest, crate::speech::SpeakResponse>(SPEAK_ID, SPEAK_DESC),
        spec::<RouterReadyEvent, ProviderReadyAck>(ON_ROUTER_READY_ID, ON_ROUTER_READY_DESC),
    ]
}
