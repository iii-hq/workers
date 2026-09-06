//! Wire-surface catalog for the `provider::elevenlabs::*` functions — the
//! single source of truth for each function's id, registration description,
//! and schemars-derived request/response schemas.
//!
//! Golden-tested in `tests/schemas.rs`; keep in lockstep with
//! [`crate::register::register_provider`]. Schema generation MUST mirror
//! iii-sdk's internal `json_schema_for` (`SchemaSettings::draft07()` on the
//! handler's request/response types) so a catalog snapshot pins exactly what
//! registration emits.

use llm_router::types::router::{
    ProviderReadyAck, RefreshModelsRequest, RefreshModelsResponse, RouterReadyEvent,
};

use crate::speech::{SpeakRequest, SpeakResponse, TranscribeRequest, TranscribeResponse};

pub const TRANSCRIBE_ID: &str = "provider::elevenlabs::transcribe";
pub const TRANSCRIBE_DESC: &str =
    "Speech to text on ElevenLabs Scribe behind router::transcribe: base64 audio in, text with \
     timed segments and the detected language out.";

pub const SPEAK_ID: &str = "provider::elevenlabs::speak";
pub const SPEAK_DESC: &str =
    "Text to speech on the ElevenLabs voices behind router::speak: text and a voice id or name \
     in, base64 audio out (mp3, wav, pcm16, opus).";

pub const REFRESH_MODELS_ID: &str = "provider::elevenlabs::refresh_models";
pub const REFRESH_MODELS_DESC: &str =
    "Refresh the ElevenLabs catalog slice from GET /v1/models (text-to-speech models with their \
     languages) plus the Scribe speech-to-text models, through router::models::reconcile.";

pub const ON_ROUTER_READY_ID: &str = "provider::elevenlabs::on_router_ready";
pub const ON_ROUTER_READY_DESC: &str =
    "Internal: re-declare with llm-router after it restarts (bound to the router::ready trigger).";

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
        spec::<TranscribeRequest, TranscribeResponse>(TRANSCRIBE_ID, TRANSCRIBE_DESC),
        spec::<SpeakRequest, SpeakResponse>(SPEAK_ID, SPEAK_DESC),
        spec::<RefreshModelsRequest, RefreshModelsResponse>(REFRESH_MODELS_ID, REFRESH_MODELS_DESC),
        spec::<RouterReadyEvent, ProviderReadyAck>(ON_ROUTER_READY_ID, ON_ROUTER_READY_DESC),
    ]
}
