//! Wire-schema snapshots for the `approval::*` functions and trigger types.
//!
//! `approval_gate::functions::catalog()` is the single source of truth for
//! each function's id, registration description, and schemars-derived
//! request/response schemas (generated with the same
//! `SchemaSettings::draft07()` construction iii-sdk uses at registration,
//! from the same input/output structs). Each entry is serialized to pretty
//! JSON and compared against `tests/golden/schemas/<id>.json` (`::` maps to
//! `.` in filenames).
//!
//! `approval_gate::functions::trigger_catalog()` covers the two custom
//! trigger types with the same pattern.
//!
//! These snapshots ARE the product surface consumed by callers and agents —
//! any schema or description change must land as an explicit golden diff.
//! Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use approval_gate::functions::{catalog, trigger_catalog, FunctionSpec, TriggerSpec};

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

fn trigger_spec_to_pretty_json(spec: &TriggerSpec) -> String {
    let value = serde_json::json!({
        "trigger_id": spec.trigger_id,
        "description": spec.description,
        "payload_schema": spec.payload_schema,
    });
    let mut pretty = serde_json::to_string_pretty(&value).expect("trigger spec serializes");
    pretty.push('\n');
    pretty
}

/// The catalog must cover exactly the 14 registered functions, in
/// registration order (kept in lockstep with `register_all`).
#[test]
fn catalog_lists_all_functions_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "approval::gate",
            "approval::resolve",
            "approval::list-pending",
            "approval::get-pending",
            "approval::set-mode",
            "approval::add-always-allow",
            "approval::remove-always-allow",
            "approval::approve-always",
            "approval::get-settings",
            "approval::clear-settings",
            "approval::on-config-change",
            "approval::on-session-deleted",
            "approval::on-turn-completed",
            "approval::sweep",
        ]
    );
}

/// The trigger catalog must cover exactly the 2 registered trigger types,
/// in registration order (kept in lockstep with `register_trigger_types`).
#[test]
fn trigger_catalog_lists_both_trigger_types_in_registration_order() {
    let ids: Vec<&str> = trigger_catalog().iter().map(|s| s.trigger_id).collect();
    assert_eq!(
        ids,
        vec!["approval::pending-created", "approval::pending-resolved",]
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

/// Every trigger catalog entry matches its committed golden.
#[test]
fn trigger_schema_snapshots_match_goldens() {
    let mut failures = Vec::new();
    for spec in trigger_catalog() {
        let rel = golden_file_name(spec.trigger_id);
        let actual = trigger_spec_to_pretty_json(&spec);
        if let Err(msg) = support::check_golden(&rel, &actual) {
            failures.push(msg);
        }
    }
    assert!(
        failures.is_empty(),
        "{} trigger wire-schema golden(s) drifted:\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Request schemas that carry field-level doc comments must keep them — a
/// rename or accidental removal of `///` comments is a breaking API change
/// even if types still compile. Only the functions whose request types
/// actually have doc-commented fields are checked here; the full wire surface
/// is already pinned by the golden snapshots above.
#[test]
fn schemas_carry_field_descriptions() {
    // Functions whose request types carry at least one "description" key in
    // their generated schema (struct-level or field-level doc comments):
    let must_have_descriptions = [
        "approval::gate",
        "approval::resolve",
        "approval::list-pending",
        "approval::get-pending",
        "approval::on-config-change",
        "approval::on-session-deleted",
        "approval::on-turn-completed",
    ];
    for spec in catalog() {
        if !must_have_descriptions.contains(&spec.function_id) {
            continue;
        }
        let rendered = serde_json::to_string(&spec.request_schema).expect("schema serializes");
        assert!(
            rendered.contains("description"),
            "{}: request schema lost its field descriptions",
            spec.function_id
        );
    }
}
