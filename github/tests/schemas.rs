//! Wire-schema snapshots for the `github::*` functions.
//!
//! `github::functions::catalog()` is the single source of truth for each
//! function's id, registration description, and schemars-derived
//! request/response schemas (generated with the same
//! `SchemaSettings::draft07()` construction iii-sdk uses at registration,
//! from the same input/output structs). Each entry is serialized to pretty
//! JSON and compared against `tests/golden/schemas/<id>.json` (`::` maps to
//! `.` in filenames).
//!
//! These snapshots ARE the product surface consumed by callers and agents —
//! any schema or description change must land as an explicit golden diff.
//! Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use github::functions::{catalog, FunctionSpec};

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

/// The catalog must cover exactly the registered functions, in registration
/// order (kept in lockstep with `register_all`; `github::on-config-change`
/// is internal and excluded).
#[test]
fn catalog_lists_all_functions_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "github::pr::list",
            "github::pr::view",
            "github::pr::create",
            "github::pr::edit",
            "github::pr::merge",
            "github::pr::comment",
            "github::pr::review",
            "github::pr::diff",
            "github::pr::checks",
            "github::issue::list",
            "github::issue::view",
            "github::issue::create",
            "github::issue::edit",
            "github::issue::comment",
            "github::issue::close",
            "github::repo::view",
            "github::repo::list",
            "github::run::list",
            "github::run::view",
            "github::run::rerun",
            "github::run::cancel",
            "github::workflow::list",
            "github::workflow::run",
            "github::release::list",
            "github::release::view",
            "github::release::create",
            "github::search::repos",
            "github::search::issues",
            "github::search::prs",
            "github::search::code",
            "github::exec",
            "github::api",
        ]
    );
}

/// Every catalog entry matches its committed golden. Mismatches are collected
/// across ALL functions before failing so one run shows the full drift, not
/// just the first file.
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

/// No function may ship the permissive `AnyValue` schema — the deploy-time
/// "unknown" request/response schema this convention exists to prevent. Every
/// request and response schema must be a typed struct.
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

/// The field docs callers rely on must appear in the schemas' doc-comment
/// descriptions (a rename here is a breaking API change even if types still
/// compile).
#[test]
fn schemas_carry_field_descriptions() {
    for spec in catalog() {
        let rendered = serde_json::to_string(&spec.request_schema).expect("schema serializes");
        assert!(
            rendered.contains("description"),
            "{}: request schema lost its field descriptions",
            spec.function_id
        );
    }
}
