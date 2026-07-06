//! Wire-schema snapshots for every function this worker registers.
//!
//! `worktree::surface::catalog()` is the single source of truth for each
//! function's id and schemars-derived request/response schemas (generated
//! with the same `SchemaSettings::draft07()` construction iii-sdk uses at
//! registration, from the same input/output structs). Each entry is
//! serialized to pretty JSON and compared against
//! `tests/golden/schemas/<id>.json` (`::` maps to `.` in filenames).
//!
//! These snapshots ARE the product surface consumed by callers and agents;
//! any schema change must land as an explicit golden diff. Regenerate with
//! `UPDATE_GOLDENS=1 cargo test`.

mod support;

use worktree::surface::{catalog, FunctionSpec};

fn golden_file_name(function_id: &str) -> String {
    format!("schemas/{}.json", function_id.replace("::", "."))
}

fn spec_to_pretty_json(spec: &FunctionSpec) -> String {
    let value = serde_json::json!({
        "function_id": spec.function_id,
        "request_schema": spec.request_schema,
        "response_schema": spec.response_schema,
    });
    let mut pretty = serde_json::to_string_pretty(&value).expect("spec serializes");
    pretty.push('\n');
    pretty
}

/// The catalog must cover exactly the registered functions, in registration
/// order (functions::register_all, then the configuration handler) — kept
/// in lockstep with the boot sequence in `main.rs`.
#[test]
fn catalog_lists_all_functions_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "worktree::create",
            "worktree::list",
            "worktree::get",
            "worktree::validate",
            "worktree::claim",
            "worktree::release",
            "worktree::status",
            "worktree::remove",
            "worktree::prune",
            "worktree::land",
            "worktree::land-step",
            "worktree::on-config-change",
        ]
    );
}

/// Every catalog entry matches its committed golden. Mismatches are
/// collected across ALL functions before failing so one run shows the full
/// drift.
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

/// No stale goldens: every file under tests/golden/schemas/ must correspond
/// to a current catalog entry.
#[test]
fn no_orphan_schema_goldens() {
    let dir = support::golden_root().join("schemas");
    let expected: Vec<String> = catalog()
        .iter()
        .map(|s| format!("{}.json", s.function_id.replace("::", ".")))
        .collect();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(
            expected.iter().any(|e| e == &name),
            "orphan golden tests/golden/schemas/{name}: no catalog entry \
             produces it. Delete it or fix the catalog."
        );
    }
}
