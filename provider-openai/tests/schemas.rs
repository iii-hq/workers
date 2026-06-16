//! Golden coverage: `provider::openai::*` publish structured wire schemas
//! instead of the permissive `AnyValue` (rendered "unknown" on the API
//! reference) that `Fn(Value) -> Value` handlers auto-extract. The published
//! types come from the shared provider protocol in `llm_router`.
use llm_router::types::router::{NoParams, ProviderAck, ProviderStreamInput, RefreshModelsAck};
use llm_router::wire_schema::schema_of;
use serde_json::Value;

fn assert_structured(schema: &Value, label: &str) {
    assert!(
        schema.is_object(),
        "{label}: schema must be a JSON object, got {schema}"
    );
    let obj = schema.as_object().unwrap();
    assert!(
        obj.contains_key("type")
            || obj.contains_key("properties")
            || obj.contains_key("oneOf")
            || obj.contains_key("anyOf"),
        "{label}: schema lacks a structural keyword: {schema}"
    );
}

#[test]
fn published_function_schemas_are_structured() {
    // provider::openai::stream
    assert_structured(&schema_of::<ProviderStreamInput>(), "stream req");
    assert_structured(&schema_of::<ProviderAck>(), "stream resp");
    // provider::openai::refresh_models
    assert_structured(&schema_of::<NoParams>(), "refresh_models req");
    assert_structured(&schema_of::<RefreshModelsAck>(), "refresh_models resp");
    // provider::openai::on_router_ready
    assert_structured(&schema_of::<ProviderAck>(), "on_router_ready resp");
}

#[test]
fn stream_request_is_the_full_provider_contract() {
    let props: Vec<String> = schema_of::<ProviderStreamInput>()
        .get("properties")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    for f in ["writer_ref", "model", "messages"] {
        assert!(
            props.contains(&f.to_string()),
            "stream req must expose `{f}`: {props:?}"
        );
    }
}
