use llm_router::types::router::{
    ProviderAbortRequest, ProviderAbortResponse, ProviderReadyAck, ProviderStreamInput,
    ProviderStreamOutput, RefreshModelsRequest, RefreshModelsResponse, RouterReadyEvent,
};

pub const STREAM_ID: &str = "provider::command-code::stream";
pub const STREAM_DESC: &str = "Stream a Command Code completion using the model's required native dialect and relay AssistantMessageEvent frames to writer_ref.";
pub const ABORT_ID: &str = "provider::command-code::abort";
pub const ABORT_DESC: &str = "Cancel the in-flight Command Code upstream stream for a request_id.";
pub const REFRESH_MODELS_ID: &str = "provider::command-code::refresh_models";
pub const REFRESH_MODELS_DESC: &str =
    "Refresh the namespaced Command Code catalog from its live public model listing.";
pub const ON_ROUTER_READY_ID: &str = "provider::command-code::on_router_ready";
pub const ON_ROUTER_READY_DESC: &str =
    "Internal router::ready subscriber that re-declares Command Code and refreshes its catalog.";

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

fn spec<Request, Response>(function_id: &'static str, description: &'static str) -> FunctionSpec
where
    Request: schemars::JsonSchema,
    Response: schemars::JsonSchema,
{
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Request>(),
        response_schema: schema_of::<Response>(),
    }
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<ProviderStreamInput, ProviderStreamOutput>(STREAM_ID, STREAM_DESC),
        spec::<ProviderAbortRequest, ProviderAbortResponse>(ABORT_ID, ABORT_DESC),
        spec::<RefreshModelsRequest, RefreshModelsResponse>(REFRESH_MODELS_ID, REFRESH_MODELS_DESC),
        spec::<RouterReadyEvent, ProviderReadyAck>(ON_ROUTER_READY_ID, ON_ROUTER_READY_DESC),
    ]
}
