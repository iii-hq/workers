mod support;

use iii_directory::surface::{search_catalog as catalog, FunctionSpec};

fn golden_file_name(function_id: &str) -> String {
    format!("schemas/{}.json", function_id.replace("::", "."))
}

fn spec_to_pretty_json(spec: &FunctionSpec) -> String {
    let value = serde_json::json!({
        "function_id": spec.function_id,
        "description": spec.description,
        "request_schema": spec.request_schema,
        "response_schema": spec.response_schema,
    });
    let mut pretty = serde_json::to_string_pretty(&value).expect("spec serializes");
    pretty.push('\n');
    pretty
}

#[test]
fn wire_schema_snapshots_match_goldens() {
    let mut failures = Vec::new();
    for spec in catalog() {
        let rel = golden_file_name(spec.function_id);
        let actual = spec_to_pretty_json(&spec);
        if let Err(msg) = support::check_golden(&rel, &actual) {
            failures.push(msg);
        }
    }
    assert!(
        failures.is_empty(),
        "{} wire-schema golden(s) drifted:\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_function_has_typed_request_and_response_schemas() {
    for spec in catalog() {
        support::assert_typed_schema(
            &format!("{} request_schema", spec.function_id),
            &spec.request_schema,
        );
        support::assert_typed_schema(
            &format!("{} response_schema", spec.function_id),
            &spec.response_schema,
        );
    }
}

#[test]
fn search_capabilities_schema_advertises_the_runtime_limit() {
    let search = catalog()
        .into_iter()
        .find(|spec| spec.function_id == "directory::search_functions")
        .expect("search function is registered");

    assert_eq!(
        search.request_schema["properties"]["capabilities"]["maxItems"],
        6
    );
    let description = search.request_schema["properties"]["capabilities"]["description"]
        .as_str()
        .expect("capabilities description is a string");
    assert!(description.contains("one search at each decision point"));
    assert!(description.contains("all unmet external capabilities"));
    assert!(description.contains("include it in `capabilities`, even if it is the only one"));
    assert!(description
        .contains("When `capabilities` is absent, the overall `query` provides fallback ranking"));
    assert!(!description.contains("Omit for a single-capability task"));
    assert!(description.contains("intrinsic reasoning, summarization, planning, or formatting"));
    assert!(description.contains("Requests to summarize provided text or content are ignored"));
    assert!(description.contains("Write every entry in English"));
    assert!(description.contains("preserving proper names, URLs, and function IDs"));
    assert!(description.contains("do not repeat needs already represented"));

    let query_description = search.request_schema["properties"]["query"]["description"]
        .as_str()
        .expect("query description is a string");
    assert!(query_description.contains("Always write it in English"));
    assert!(query_description.contains("preserving proper names, URLs, and function IDs"));
}
