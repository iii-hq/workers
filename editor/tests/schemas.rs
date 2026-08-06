//! Wire-schema snapshots for every registered `editor::*` function.
//!
//! `editor::surface::catalog()` is the single source of truth for each
//! function's id, registration description, and schemars-derived
//! request/response schemas (generated with the same
//! `SchemaSettings::draft07()` construction iii-sdk uses at registration, from
//! the same input/output structs). Each entry is serialized to pretty JSON and
//! compared against `tests/golden/schemas/<id>.json` (`::` maps to `.`).
//!
//! These snapshots ARE the product surface consumed by callers and agents —
//! any schema or description change must land as an explicit golden diff.
//! Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use editor::functions::function_ids;
use editor::surface::{catalog, FunctionSpec};

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
/// order — `function_ids()` is what `register_all` walks.
#[test]
fn catalog_matches_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(ids, function_ids());
}

/// Every catalog entry matches its committed golden. Mismatches are collected
/// across ALL functions before failing so one run shows the full drift.
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

/// Field doc comments are the only documentation an agent sees at call time.
///
/// Parameterless requests are exempt: `editor::workspace::get` and
/// `editor::buffers::list` take `{}`, and there is no field there to document.
/// The registration description carries their meaning instead, which the
/// golden snapshot pins.
#[test]
fn schemas_with_fields_carry_field_descriptions() {
    for spec in catalog() {
        let rendered = serde_json::to_string(&spec.request_schema).expect("schema serializes");
        if !rendered.contains("properties") {
            continue;
        }
        assert!(
            rendered.contains("description"),
            "{}: request schema lost its field descriptions",
            spec.function_id
        );
    }
}

/// Every function must carry a registration description — for the empty-input
/// ones above it is the only documentation an agent gets.
#[test]
fn every_function_has_a_description() {
    for spec in catalog() {
        assert!(
            spec.description.len() > 20,
            "{}: registration description is missing or too short to be useful",
            spec.function_id
        );
    }
}
