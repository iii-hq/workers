//! Wire-schema snapshot for the public `rbac-proxy::*` catalog.
//!
//! `rbac_proxy::functions::catalog()` is the single source of truth for each
//! public function's id, registration description, and schemars-derived
//! request/response schemas (same `SchemaSettings::draft07()` construction
//! iii-sdk uses at registration). Each entry is serialized to pretty JSON and
//! compared against `tests/golden/schemas/<id>.json` (`::` maps to `.`).
//!
//! The internal `rbac-proxy::on-config-change` / `rbac-proxy::on-functions-
//! available` handlers are registered off this public catalog (in
//! `configuration.rs`), mirroring approval-gate / context-manager, so they are
//! not snapshotted here.
//!
//! Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use rbac_proxy::functions::{catalog, FunctionSpec};

fn golden_file_name(id: &str) -> String {
    format!("schemas/{}.json", id.replace("::", "."))
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

/// The catalog must cover exactly the one public function, in registration
/// order (kept in lockstep with `register_all`).
#[test]
fn catalog_lists_public_functions_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(ids, vec!["rbac-proxy::status"]);
}

/// Every catalog entry matches its committed golden.
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

/// No function may ship the permissive `AnyValue` schema.
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
