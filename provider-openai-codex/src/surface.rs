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

pub const STREAM_ID: &str = "provider::openai-codex::stream";
pub const STREAM_DESC: &str = "Stream an OpenAI Codex completion: resolve a ChatGPT OAuth token \
     from the provider session, call the upstream Responses API, and relay AssistantMessageEvent frames to \
     writer_ref.";

pub const ABORT_ID: &str = "provider::openai-codex::abort";
pub const ABORT_DESC: &str = "Cancel the in-flight upstream stream for a request_id \
     (router::abort fan-out), stopping billed generation immediately.";

pub const REFRESH_MODELS_ID: &str = "provider::openai-codex::refresh_models";
pub const REFRESH_MODELS_DESC: &str = "Fetch the authenticated Codex model catalog and replace \
     this provider's namespaced router slice; returns the active model count.";

pub const ON_ROUTER_READY_ID: &str = "provider::openai-codex::on_router_ready";
pub const ON_ROUTER_READY_DESC: &str =
    "Internal: router::ready subscriber that re-declares this provider and refreshes its catalog.";

pub const COUNT_TOKENS_ID: &str = "provider::openai-codex::count_tokens";
pub const COUNT_TOKENS_DESC: &str =
    "Count prompt tokens for {model, system_prompt?, tools?, messages} locally with the \
     tiktoken tokenizers; never runs the model and costs nothing.";

pub const LOGIN_START_ID: &str = "provider::openai-codex::login::start";
pub const LOGIN_START_DESC: &str = "Start a ChatGPT device-code login. Returns the verification URL and one-time user code; the provider completes and saves the session in the background.";
pub const LOGIN_POLL_ID: &str = "provider::openai-codex::login::poll";
pub const LOGIN_POLL_DESC: &str =
    "Read a device login attempt's status without exposing OAuth tokens.";
pub const LOGIN_CANCEL_ID: &str = "provider::openai-codex::login::cancel";
pub const LOGIN_CANCEL_DESC: &str =
    "Cancel a pending device login. Does not disconnect the current account.";
pub const AUTH_STATUS_ID: &str = "provider::openai-codex::auth::status";
pub const AUTH_STATUS_DESC: &str = "Read Codex session status, credential source, account id, and any pending login. Never returns tokens.";
pub const AUTH_LOGOUT_ID: &str = "provider::openai-codex::auth::logout";
pub const AUTH_LOGOUT_DESC: &str = "Disconnect this provider, cancel pending login, and persistently disable automatic use of legacy credentials. Does not sign out the Codex CLI.";

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
        spec::<crate::count_tokens::CountTokensRequest, crate::count_tokens::CountTokensResponse>(
            COUNT_TOKENS_ID,
            COUNT_TOKENS_DESC,
        ),
        spec::<crate::session::EmptyRequest, crate::session::LoginStartResponse>(
            LOGIN_START_ID,
            LOGIN_START_DESC,
        ),
        spec::<crate::session::LoginIdRequest, crate::session::LoginPollResponse>(
            LOGIN_POLL_ID,
            LOGIN_POLL_DESC,
        ),
        spec::<crate::session::LoginIdRequest, crate::session::Ack>(
            LOGIN_CANCEL_ID,
            LOGIN_CANCEL_DESC,
        ),
        spec::<crate::session::EmptyRequest, crate::session::AuthStatusResponse>(
            AUTH_STATUS_ID,
            AUTH_STATUS_DESC,
        ),
        spec::<crate::session::EmptyRequest, crate::session::Ack>(AUTH_LOGOUT_ID, AUTH_LOGOUT_DESC),
    ]
}
