//! Golden coverage for the published `router::*` wire schemas.
//!
//! Regression guard: the router's functions were registered with
//! `Fn(Value) -> Value` handlers, so the SDK auto-extracted the permissive
//! `AnyValue` schema (which serializes to the JSON literal `true`) and the
//! workers.iii.dev API reference rendered "unknown". These tests assert every
//! request/response type produces a *structured* schema instead.
use llm_router::types::router::{
    AbortRequest, AbortResponse, ChatRequest, ChatResponse, CompleteRequest, CompleteResponse,
    ConfigChangedEvent, ModelsGetRequest, ModelsGetResponse, ModelsListRequest, ModelsListResponse,
    ModelsReconcileRequest, ModelsReconcileResponse, ModelsSupportsRequest, ModelsSupportsResponse,
    OkResponse, ProviderAck, ProviderListRequest, ProviderListResponse, ProviderRegisterResponse,
    ProviderResolveRequest, ProviderResolveResponse, ProviderStreamInput, RefreshModelsAck,
    RouteRequest, RouteResponse, UpdateCredentialRequest, WorkerAvailableEvent,
};
use llm_router::wire_schema::schema_of;
use schemars::JsonSchema;
use serde_json::Value;

/// A structured schema is a JSON object with a recognizable schema keyword —
/// never the bare `true` that `serde_json::Value` (AnyValue) produces.
fn assert_structured(schema: &Value, label: &str) {
    assert!(
        schema.is_object(),
        "{label}: schema must be a JSON object, got {schema} (AnyValue/`true` means the handler is still untyped)"
    );
    let obj = schema.as_object().unwrap();
    let has_shape = obj.contains_key("type")
        || obj.contains_key("properties")
        || obj.contains_key("oneOf")
        || obj.contains_key("anyOf")
        || obj.contains_key("enum")
        || obj.contains_key("$ref");
    assert!(
        has_shape,
        "{label}: schema lacks any structural keyword: {schema}"
    );
}

fn props(schema: &Value) -> Vec<String> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

fn schema_named<T: JsonSchema>(label: &str) -> Value {
    let s = schema_of::<T>();
    assert_structured(&s, label);
    s
}

#[test]
fn every_request_and_response_type_publishes_a_structured_schema() {
    // request types
    schema_named::<ChatRequest>("ChatRequest");
    schema_named::<CompleteRequest>("CompleteRequest");
    schema_named::<AbortRequest>("AbortRequest");
    schema_named::<RouteRequest>("RouteRequest");
    schema_named::<ModelsListRequest>("ModelsListRequest");
    schema_named::<ModelsGetRequest>("ModelsGetRequest");
    schema_named::<ModelsSupportsRequest>("ModelsSupportsRequest");
    schema_named::<ProviderListRequest>("ProviderListRequest");
    schema_named::<ProviderResolveRequest>("ProviderResolveRequest");
    schema_named::<UpdateCredentialRequest>("UpdateCredentialRequest");
    schema_named::<ModelsReconcileRequest>("ModelsReconcileRequest");
    schema_named::<WorkerAvailableEvent>("WorkerAvailableEvent");
    schema_named::<ConfigChangedEvent>("ConfigChangedEvent");
    schema_named::<ProviderStreamInput>("ProviderStreamInput");

    // response types
    schema_named::<ChatResponse>("ChatResponse");
    schema_named::<CompleteResponse>("CompleteResponse");
    schema_named::<AbortResponse>("AbortResponse");
    schema_named::<RouteResponse>("RouteResponse");
    schema_named::<ModelsListResponse>("ModelsListResponse");
    schema_named::<ModelsGetResponse>("ModelsGetResponse");
    schema_named::<ModelsSupportsResponse>("ModelsSupportsResponse");
    schema_named::<ProviderListResponse>("ProviderListResponse");
    schema_named::<ProviderRegisterResponse>("ProviderRegisterResponse");
    schema_named::<ProviderResolveResponse>("ProviderResolveResponse");
    schema_named::<ModelsReconcileResponse>("ModelsReconcileResponse");
    schema_named::<OkResponse>("OkResponse");
    schema_named::<ProviderAck>("ProviderAck");
    schema_named::<RefreshModelsAck>("RefreshModelsAck");
}

#[test]
fn chat_request_carries_the_streaming_writer_ref() {
    let p = props(&schema_of::<ChatRequest>());
    for field in ["writer_ref", "model", "messages"] {
        assert!(
            p.contains(&field.to_string()),
            "ChatRequest must expose `{field}`: {p:?}"
        );
    }
}

#[test]
fn complete_request_is_chat_without_writer_ref() {
    // `router::complete` owns an internal channel, so its published request
    // must NOT advertise a caller-supplied writer_ref.
    let p = props(&schema_of::<CompleteRequest>());
    assert!(
        p.contains(&"model".to_string()),
        "CompleteRequest must expose `model`: {p:?}"
    );
    assert!(
        p.contains(&"messages".to_string()),
        "CompleteRequest must expose `messages`: {p:?}"
    );
    assert!(
        !p.contains(&"writer_ref".to_string()),
        "CompleteRequest must NOT advertise writer_ref: {p:?}"
    );
}

#[test]
fn provider_stream_input_is_the_full_provider_contract() {
    let p = props(&schema_of::<ProviderStreamInput>());
    for field in ["writer_ref", "model", "messages"] {
        assert!(
            p.contains(&field.to_string()),
            "ProviderStreamInput must expose `{field}`: {p:?}"
        );
    }
}

#[test]
fn unit_response_schema_is_null_typed_not_anyvalue() {
    // The trigger handlers (`on_worker_available`, `on_config_changed`) return
    // Value::Null; their published response is the `()` schema.
    let s = schema_of::<()>();
    assert_eq!(
        s.get("type"),
        Some(&Value::String("null".into())),
        "unit schema must be null-typed: {s}"
    );
}
