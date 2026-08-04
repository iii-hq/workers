//! Wire-schema snapshots for the five `pdf::*` functions.
//!
//! `pdf::functions::catalog()` is the single source of truth for each
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

use pdf::functions::{catalog, FunctionSpec};

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
fn catalog_lists_all_five_functions_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "pdf::classify",
            "pdf::to-markdown",
            "pdf::extract-text",
            "pdf::extract-items",
            "pdf::extract-regions",
        ]
    );
}

/// Every catalog entry matches its committed golden. Mismatches are collected
/// across ALL functions before failing, so one run shows the full drift.
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

/// The page-numbering convention is the easiest thing on this surface to get
/// silently wrong, so it must be stated in the schema an agent reads, not only
/// in the worker README.
#[test]
fn page_fields_state_their_indexing() {
    for spec in catalog() {
        let rendered = serde_json::to_string(&spec.response_schema).expect("schema serializes");
        if rendered.contains("\"page\"") || rendered.contains("pages_needing_ocr") {
            assert!(
                rendered.contains("1-indexed"),
                "{}: response mentions pages without stating the indexing",
                spec.function_id
            );
        }
    }
}

/// Both coordinate conventions must be spelled out where they are used. The two
/// functions disagree on the origin, and a caller that assumes wrong gets text
/// from the wrong end of the page with no error.
#[test]
fn coordinate_conventions_are_documented_on_the_wire() {
    for spec in catalog() {
        let rendered = serde_json::to_string(&spec.response_schema).expect("schema serializes");
        match spec.function_id {
            "pdf::extract-items" => assert!(
                rendered.contains("bottom"),
                "extract-items must document its bottom-left origin"
            ),
            "pdf::extract-regions" => assert!(
                rendered.contains("top left") || rendered.contains("top-left"),
                "extract-regions must document its top-left origin"
            ),
            _ => {}
        }
    }
}
