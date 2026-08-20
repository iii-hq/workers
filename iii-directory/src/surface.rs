//! The search surface as data: ids, descriptions, and typed
//! request/response schemas — the single source for registration and the
//! golden schema tests.

use crate::functions::search::{
    AckResponse, OnFunctionsChangeEvent, SearchFunctionsRequest, SearchFunctionsResponse,
};
use crate::hook::{
    HintPreviewRequest, HintPreviewResponse, PreGenerateHookRequest, PreGenerateHookResponse,
};

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: serde_json::Value,
    pub response_schema: serde_json::Value,
}

fn schema<T: schemars::JsonSchema>() -> serde_json::Value {
    let root = schemars::gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>();
    serde_json::to_value(root).expect("function schema serializes")
}

fn spec<I: schemars::JsonSchema, O: schemars::JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema::<I>(),
        response_schema: schema::<O>(),
    }
}

pub fn search_catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<SearchFunctionsRequest, SearchFunctionsResponse>(
            "directory::search_functions",
            "Search every available function with one natural-language query describing everything the task needs; returns the full API reference for each relevant function, grouped by worker.",
        ),
        spec::<PreGenerateHookRequest, PreGenerateHookResponse>(
            "directory::pre-generate",
            "Internal: inject the conditional search hint into one harness generation.",
        ),
        spec::<OnFunctionsChangeEvent, AckResponse>(
            "directory::on-functions-change",
            "Internal: refresh the search catalog after the engine function set changes.",
        ),
        spec::<HintPreviewRequest, HintPreviewResponse>(
            "directory::hint-preview",
            "Internal: the exact search-hint text per exposure mode, for the configuration UI.",
        ),
    ]
}
