//! Wire-schema snapshots for the `iii-cloud::*` functions. `catalog()` is the
//! single source of truth for each function's id, description and schemas;
//! each entry is compared with `tests/golden/schemas/<id>.json` (`::` is `.`).
//! Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use iii_cloud::functions::{FunctionSpec, catalog};

fn golden_file_name(function_id: &str) -> String {
    format!("schemas/{}.json", function_id.replace("::", "."))
}

fn pretty(spec: &FunctionSpec) -> String {
    let value = serde_json::json!({
        "function_id": spec.function_id,
        "description": spec.description,
        "request_schema": spec.request_schema,
        "response_schema": spec.response_schema,
    });
    let mut out = serde_json::to_string_pretty(&value).unwrap();
    out.push('\n');
    out
}

#[test]
fn catalog_lists_the_public_ids_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "iii-cloud::connect",
            "iii-cloud::status",
            "iii-cloud::project::resolve",
            "iii-cloud::project::use",
            "iii-cloud::project::create",
            "iii-cloud::project::create_environment",
            "iii-cloud::api",
            "iii-cloud::api::write",
            "iii-cloud::check",
            "iii-cloud::check::get",
            "iii-cloud::secret",
            "iii-cloud::secret::provide",
            "iii-cloud::deploy",
            "iii-cloud::watch",
            "iii-cloud::handoff",
            "iii-cloud::handoff::stop",
            "iii-cloud::handoff::progress",
            "iii-cloud::handoff::worktree",
            "iii-cloud::values",
            "iii-cloud::values::save",
            "iii-cloud::values::skip",
            "iii-cloud::problem::skip",
            "iii-cloud::config::import",
            "iii-cloud::override::choose",
            "iii-cloud::on-change",
        ]
    );
}

#[test]
fn wire_schema_snapshots_match_goldens() {
    let failures: Vec<String> = catalog()
        .iter()
        .filter_map(|s| support::check_golden(&golden_file_name(s.function_id), &pretty(s)).err())
        .collect();
    assert!(
        failures.is_empty(),
        "{} golden(s) drifted:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_function_has_typed_schemas() {
    for s in catalog() {
        support::assert_typed_schema(&format!("{} request", s.function_id), &s.request_schema);
        support::assert_typed_schema(&format!("{} response", s.function_id), &s.response_schema);
    }
}

/// The key connect path takes the secret's name alone; the ids are a fallback.
#[test]
fn connect_api_key_ids_are_optional() {
    let connect = &catalog()[0];
    let text = serde_json::to_string(&connect.request_schema).unwrap();
    assert!(text.contains("\"secret\""));
    let api_key = connect
        .request_schema
        .schema
        .subschemas
        .as_ref()
        .and_then(|s| s.one_of.as_ref())
        .unwrap()
        .iter()
        .map(|s| serde_json::to_value(s).unwrap())
        .find(|v| v.pointer("/properties/secret").is_some())
        .unwrap();
    assert_eq!(api_key["required"], serde_json::json!(["method", "secret"]));
}
