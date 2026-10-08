//! Wire-schema snapshots for the ten `computer::*` functions.
//!
//! `computer::functions::catalog()` is the single source of truth for each
//! function's id, registration description, and schemars-derived
//! request/response schemas (generated with the same `SchemaSettings::draft07()`
//! construction iii-sdk uses at registration, from the same input/output
//! structs). Each entry is serialized to pretty JSON and compared against
//! `tests/golden/schemas/<id>.json` (`::` maps to `.` in filenames).
//!
//! These snapshots ARE the product surface consumed by callers and agents; any
//! schema or description change must land as an explicit golden diff.
//! Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use computer::functions::{catalog, FunctionSpec};

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

/// The catalog must cover exactly the ten registered functions, in
/// registration order (kept in lockstep with `register_all`).
#[test]
fn catalog_lists_all_functions_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "computer::sessions::start",
            "computer::sessions::list",
            "computer::sessions::stop",
            "computer::displays",
            "computer::screenshot",
            "computer::observe",
            "computer::act",
            "computer::screencast::start",
            "computer::screencast::stop",
            "computer::frame",
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

/// No function may ship the permissive `AnyValue` schema (the deploy-time
/// "unknown" request/response schema this convention exists to prevent). Every
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

/// The worker-owned `computer::frame-changed` trigger type: binding config and
/// notification payload are wire surface too (the console viewport binds it).
#[test]
fn frame_changed_trigger_contract_matches_golden() {
    use computer::frames::{FrameBindingConfig, FrameChange, FRAME_CHANGED, FRAME_CHANGED_DESC};
    let generator = || schemars::r#gen::SchemaSettings::draft07().into_generator();
    let value = serde_json::json!({
        "trigger_type": FRAME_CHANGED,
        "description": FRAME_CHANGED_DESC,
        "config_schema": generator().into_root_schema_for::<FrameBindingConfig>(),
        "payload_schema": generator().into_root_schema_for::<FrameChange>(),
    });
    let mut pretty = serde_json::to_string_pretty(&value).expect("contract serializes");
    pretty.push('\n');
    if let Err(msg) = support::check_golden("triggers/computer.frame-changed.json", &pretty) {
        panic!("{msg}");
    }
}
