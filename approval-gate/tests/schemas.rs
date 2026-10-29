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

use approval_gate::functions::{
    catalog, observable_catalog, trigger_catalog, FunctionSpec, ObservableSpec, TriggerSpec,
};

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

/// The catalog must cover exactly the 13 registered functions, in
/// registration order (kept in lockstep with `register_all`). The internal
/// `approval::on-config-change` handler is registered separately via
/// `configuration::register_config_trigger` (mirroring session-manager /
/// context-manager), so it is not part of this public catalog.
#[test]
fn catalog_lists_all_functions_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "approval::gate",
            "approval::filesystem-access-watch",
            "approval::resolve",
            "approval::evaluate",
            "approval::list-pending",
            "approval::get-pending",
            "approval::set-mode",
            "approval::add-always-allow",
            "approval::remove-always-allow",
            "approval::approve-always",
            "approval::get-settings",
            "approval::clear-settings",
            "approval::on-session-deleted",
            "approval::on-turn-completed",
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

fn observable_spec_to_pretty_json(spec: &ObservableSpec) -> String {
    let value = serde_json::json!({
        "function_id": spec.function_id,
        "description": spec.description,
        "args_schema": spec.args_schema,
        "event_schema": spec.event_schema,
    });
    let mut pretty = serde_json::to_string_pretty(&value).expect("observable spec serializes");
    pretty.push('\n');
    pretty
}

/// The observable functions (iii 0.25): one next to each trigger type, in
/// the trigger catalog's order (kept in lockstep with
/// `register_trigger_types`).
#[test]
fn observable_catalog_lists_one_function_next_to_each_trigger_type() {
    let ids: Vec<&str> = observable_catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "approval::on-pending-created",
            "approval::on-pending-resolved"
        ]
    );
    for (observable, trigger) in observable_catalog().iter().zip(trigger_catalog()) {
        assert_eq!(
            observable.function_id,
            trigger
                .trigger_id
                .replacen("approval::", "approval::on-", 1)
        );
        // Observers receive exactly what a binding of the trigger type receives.
        assert_eq!(
            serde_json::to_value(&observable.event_schema).unwrap(),
            serde_json::to_value(&trigger.payload_schema).unwrap(),
            "{}: event schema differs from {}'s payload",
            observable.function_id,
            trigger.trigger_id
        );
        // Observing is refused (FORBIDDEN) when invoked: documented.
        assert!(
            observable.description.contains("refused"),
            "{}: description must say direct invocation is refused",
            observable.function_id
        );
    }
}

/// Both observable functions take the binding config as args.
#[test]
fn observable_args_are_the_binding_config() {
    for spec in observable_catalog() {
        let args = serde_json::to_value(&spec.args_schema).unwrap();
        let properties = args["properties"].as_object().expect("object args");
        assert!(
            properties.contains_key("session_id"),
            "{}",
            spec.function_id
        );
        assert!(properties.contains_key("metadata"), "{}", spec.function_id);
        assert_eq!(args["additionalProperties"], serde_json::json!(false));
    }
}

/// Every observable function matches its committed golden.
#[test]
fn observable_schema_snapshots_match_goldens() {
    let mut failures = Vec::new();
    for spec in observable_catalog() {
        let rel = golden_file_name(spec.function_id);
        let actual = observable_spec_to_pretty_json(&spec);
        if let Err(msg) = support::check_golden(&rel, &actual) {
            failures.push(msg);
        }
    }
    assert!(
        failures.is_empty(),
        "{} observable wire-schema golden(s) drifted:\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// No function may ship the permissive `AnyValue` schema — the deploy-time
/// "unknown" request/response schema this convention exists to prevent. Every
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
    // The observable `approval::on-*` functions register their handler on
    // the delivered event: that event schema is their request schema.
    for spec in observable_catalog() {
        support::assert_typed_schema(
            &format!("{} request_schema", spec.function_id),
            &spec.event_schema,
        );
    }
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
        "approval::filesystem-access-watch",
        "approval::resolve",
        "approval::evaluate",
        "approval::list-pending",
        "approval::get-pending",
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
