mod support;

use a2ui::functions::{catalog, FunctionSpec};

fn file_name(function_id: &str) -> String {
    format!("schemas/{}.json", function_id.replace("::", "."))
}

fn pretty(spec: &FunctionSpec) -> String {
    let value = serde_json::json!({
        "function_id": spec.function_id,
        "description": spec.description,
        "request_schema": spec.request_schema,
        "response_schema": spec.response_schema,
    });
    let mut output = serde_json::to_string_pretty(&value).expect("spec serializes");
    output.push('\n');
    output
}

#[test]
fn wire_schema_snapshots_match_goldens() {
    assert!(
        !(std::env::var_os("CI").is_some() && std::env::var_os("UPDATE_GOLDENS").is_some()),
        "UPDATE_GOLDENS must not be set in CI"
    );
    let failures: Vec<String> = catalog()
        .iter()
        .filter_map(|spec| support::check_golden(&file_name(spec.function_id), &pretty(spec)).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_registered_function_has_typed_schemas_and_descriptions() {
    for spec in catalog() {
        support::assert_typed_schema(
            &format!("{} request_schema", spec.function_id),
            &spec.request_schema,
        );
        support::assert_typed_schema(
            &format!("{} response_schema", spec.function_id),
            &spec.response_schema,
        );
        let request = serde_json::to_string(&spec.request_schema).unwrap();
        assert!(
            request.contains("description"),
            "{} lost request docs",
            spec.function_id
        );
    }
}
