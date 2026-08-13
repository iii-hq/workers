//! Wire-schema snapshots for the seven `canvas::*` functions.
//!
//! `canvas::functions::catalog()` is the single source of truth for each
//! function's id, registration description, and schemars-derived request and
//! response schemas, generated with the same construction iii-sdk uses at
//! registration, from the same input and output structs. Each entry is
//! serialized to pretty JSON and compared against
//! `tests/golden/schemas/<id>.json` (`::` maps to `.` in filenames).
//!
//! These snapshots ARE the product surface consumed by callers and agents, so
//! any schema or description change must land as an explicit golden diff.
//! Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use canvas::functions::{catalog, FunctionSpec};

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
/// order (kept in lockstep with `register_all`).
#[test]
fn catalog_lists_every_function_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "canvas::create",
            "canvas::get",
            "canvas::list",
            "canvas::update",
            "canvas::delete",
            "canvas::syntax",
            "canvas::validate",
            "canvas::element::add",
            "canvas::element::update",
            "canvas::element::delete",
            "canvas::element::list",
        ]
    );
}

/// Every catalog entry matches its committed golden. Mismatches are collected
/// across ALL functions before failing, so one run shows the full drift.
#[test]
fn wire_schema_snapshots_match_goldens() {
    assert!(
        !(std::env::var_os("CI").is_some() && std::env::var_os("UPDATE_GOLDENS").is_some()),
        "UPDATE_GOLDENS must not be set in CI — goldens are committed, never regenerated there"
    );
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
/// "unknown" request/response schema this convention exists to prevent.
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

/// Field doc comments become schema descriptions, and callers rely on them.
/// Losing them is a silent documentation regression that still compiles.
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

/// The record's `id` is the one field every caller chains on, and its
/// stability across updates is the property that makes chaining safe — it
/// must be stated in the schema an agent reads, not only in the README.
#[test]
fn record_bearing_schemas_state_id_stability() {
    for spec in catalog() {
        let rendered = serde_json::to_string(&spec.response_schema).expect("schema serializes");
        if rendered.contains("created_at") {
            assert!(
                rendered.contains("8-character"),
                "{}: response carries a canvas record but does not describe the stable \
                 8-character id",
                spec.function_id
            );
        }
    }
}
