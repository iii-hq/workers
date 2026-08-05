//! Wire-schema snapshots for the statically registered `code-runner::*`
//! functions. `sandbox_code_runner::functions::catalog()` is the single source of
//! truth; each entry is serialized to pretty JSON and compared against
//! `tests/golden/schemas/<id>.json` (`::` maps to `.` in filenames).
//!
//! Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use sandbox_code_runner::functions::{catalog, FunctionSpec};

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
fn catalog_lists_every_function_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "code-runner::eval",
            "code-runner::teardown",
            "code-runner::register_function",
            "code-runner::inject-guidance"
        ]
    );
}

#[test]
fn wire_schema_snapshots_match_goldens() {
    let mut failures = Vec::new();
    for spec in catalog() {
        let rel = golden_file_name(spec.function_id);
        if let Err(msg) = support::check_golden(&rel, &spec_to_pretty_json(&spec)) {
            failures.push(msg);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// A `serde_json::Value` handler emits the permissive AnyValue schema, which
/// ships to the registry as "unknown". Every static function must be typed.
#[test]
fn every_schema_is_typed() {
    for spec in catalog() {
        for (kind, schema) in [
            ("request", &spec.request_schema),
            ("response", &spec.response_schema),
        ] {
            support::assert_typed_schema(&format!("{} {kind}", spec.function_id), schema);
        }
    }
}

/// The two hand-maintained lists must not drift apart. Compared as ordered
/// slices so an id appended to one list and inserted into the other fails
/// in CI, not just at deploy-time via register_all's assert.
#[test]
fn static_ids_and_catalog_match_exactly() {
    let cataloged: Vec<&str> = sandbox_code_runner::functions::catalog()
        .iter()
        .map(|s| s.function_id)
        .collect();
    assert_eq!(
        sandbox_code_runner::functions::STATIC_IDS,
        cataloged.as_slice(),
        "STATIC_IDS and catalog() must list the same ids in the same order"
    );
}
