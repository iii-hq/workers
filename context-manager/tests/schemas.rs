//! Wire-schema snapshots for the four `context::*` functions.
//!
//! `context_manager::functions::catalog()` is the single source of
//! truth for each function's id, registration description, and
//! schemars-derived request/response schemas (generated with the same
//! `SchemaSettings::draft07()` construction iii-sdk uses at
//! registration, from the same input/output structs). Each entry is
//! serialized to pretty JSON and compared against
//! `tests/golden/schemas/<id>.json` (`::` maps to `.` in filenames).
//!
//! These snapshots ARE the product surface consumed by callers and
//! agents — any schema or description change must land as an explicit
//! golden diff. Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use context_manager::functions::{catalog, FunctionSpec};

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

/// The catalog must cover exactly the four registered functions, in
/// registration order (kept in lockstep with `register_all`).
#[test]
fn catalog_lists_all_four_functions_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "context::assemble",
            "context::compact",
            "context::prune",
            "context::count_tokens",
        ]
    );
}

/// Every catalog entry matches its committed golden. Mismatches are
/// collected across ALL functions before failing so one run shows the
/// full drift, not just the first file.
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

/// The spec strings callers are documented to rely on must appear in
/// the schemas' doc-comment descriptions (a rename here is a breaking
/// API change even if types still compile).
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
