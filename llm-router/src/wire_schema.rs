//! Published wire schemas for the `router::*` function surface.
//!
//! Why this exists: the router's handlers are deliberately tolerant — they
//! accept `serde_json::Value`, apply defaults, forward `messages` verbatim,
//! own a streaming `writer_ref` sink, and (for `models::get`) answer with a
//! bare `null`. None of that survives a strict typed `Fn(Req) -> Resp`
//! signature without changing the wire contract. So instead of retyping the
//! handlers, we keep them on `Value` and attach precise request/response JSON
//! Schemas explicitly via [`RegisterFunction::request_format`] /
//! [`response_format`]. Without this the SDK auto-extracts the permissive
//! `AnyValue` schema from `Fn(Value) -> Value`, which renders as "unknown" on
//! the workers.iii.dev API reference.
use iii_sdk::RegisterFunction;
use schemars::JsonSchema;
use serde_json::Value;

/// Draft-07 JSON Schema for `T`.
///
/// Generator settings mirror iii-sdk's internal `json_schema_for`
/// (`SchemaSettings::draft07()`), so an explicit override here is byte-for-byte
/// what a typed handler of the same type would have auto-extracted.
pub fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(
        schemars::r#gen::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<T>(),
    )
    .expect("a JsonSchema type always serializes to a JSON value")
}

/// Attach precise request/response schemas to a registration whose handler
/// stays `Fn(Value) -> Value`. Dispatch is unchanged; only the published
/// schema surface gains structure.
pub fn with_schemas<Req: JsonSchema, Resp: JsonSchema>(f: RegisterFunction) -> RegisterFunction {
    f.request_format(schema_of::<Req>())
        .response_format(schema_of::<Resp>())
}
