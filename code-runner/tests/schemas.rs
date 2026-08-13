//! Wire-schema snapshots for the statically registered `code-runner::*`
//! functions. `code_runner::functions::catalog()` is the single source of
//! truth; each entry is serialized to pretty JSON and compared against
//! `tests/golden/schemas/<id>.json` (`::` maps to `.` in filenames).
//!
//! Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use code_runner::functions::{catalog, FunctionSpec};

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
            "code-runner::run",
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

/// `STATIC_IDS` seeds the IdRegistry; anything this worker registers but does
/// not seed can reach the SDK's duplicate-id panic, which aborts the process.
///
/// What this proves, and only this: the two hand-maintained lists have not
/// drifted APART. An id missing from BOTH leaves them equal and passes —
/// neither direction catches a function added to `register_all` alone. That
/// case is covered where it can be: `register_all` asserts it registered
/// exactly `STATIC_IDS`.
///
/// Compared as ordered slices, not sets. `register_all`'s assertion is
/// order-sensitive but runs only at startup, where a mismatch is a failed
/// deploy; pinning the order here too means the likeliest drift — a new
/// function appended to one list and inserted into the other — fails in CI
/// instead.
#[test]
fn static_ids_and_catalog_match_exactly() {
    let cataloged: Vec<&str> = code_runner::functions::catalog()
        .iter()
        .map(|s| s.function_id)
        .collect();
    assert_eq!(
        code_runner::functions::STATIC_IDS,
        cataloged.as_slice(),
        "STATIC_IDS and catalog() must list the same ids in the same order — an \
         unseeded registration reopens the duplicate-id process abort (see crate::ids)"
    );
}

/// The four ids have to agree across the catalog, `STATIC_IDS` and the golden
/// filenames. Any one out of step is a worker that advertises an id it does
/// not serve — and `sandbox-code-runner`'s own history is why this is pinned:
/// this worker's name was once its name, and `::eval` was once `::run`.
#[test]
fn the_advertised_ids_agree_across_catalog_static_ids_and_goldens() {
    let catalog = code_runner::functions::catalog();
    assert!(catalog.iter().any(|s| s.function_id == "code-runner::run"));
    assert!(!catalog.iter().any(|s| s.function_id == "code-runner::eval"));
    assert!(code_runner::functions::STATIC_IDS.contains(&"code-runner::run"));
    assert!(std::path::Path::new("tests/golden/schemas/code-runner.run.json").exists());
    assert!(!std::path::Path::new("tests/golden/schemas/code-runner.eval.json").exists());
}

/// A half-finished removal — the id gone from `STATIC_IDS` but the function
/// still registered, or a stale golden left on disk — passes every other test
/// in this file, because they compare two hand-maintained lists to each other.
#[test]
fn removed_functions_are_absent_from_the_catalog() {
    let catalog = code_runner::functions::catalog();
    // Ids this worker deliberately does not serve: node-engine's retired
    // pair, and the `::eval` spelling that predates `::run`.
    for gone in [
        "code-runner::eject",
        "code-runner::register_worker",
        "code-runner::eval",
    ] {
        assert!(
            !catalog.iter().any(|s| s.function_id == gone),
            "{gone} is still in the catalog"
        );
        assert!(
            !code_runner::functions::STATIC_IDS.contains(&gone),
            "{gone} is still in STATIC_IDS"
        );
        let golden = format!("tests/golden/schemas/{}.json", gone.replace("::", "."));
        assert!(
            !std::path::Path::new(&golden).exists(),
            "{golden} still exists on disk"
        );
    }
}
