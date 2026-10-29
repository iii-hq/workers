//! Wire-schema snapshots for the twelve `computer::*` functions.
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

/// The catalog must cover exactly the twelve registered functions, in
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
            "computer::find",
            "computer::act",
            "computer::run",
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

/// The four observable functions this worker provides (the console binds
/// them): each one's args (`null`: it takes none) and the event its handler
/// returns, which observers observe, are wire surface too.
#[test]
fn event_functions_match_golden() {
    use computer::events::{
        ActivityEvent, SessionStartedEvent, SessionStoppedEvent, ACTIVITY_DESC, ON_ACTIVITY_FN,
        ON_SESSION_STARTED_FN, ON_SESSION_STOPPED_FN, SESSION_STARTED_DESC, SESSION_STOPPED_DESC,
    };
    use computer::frames::{FrameArgs, FrameChange, ON_FRAME_CHANGED_DESC, ON_FRAME_CHANGED_FN};

    fn schema<T: schemars::JsonSchema>() -> serde_json::Value {
        let generator = schemars::r#gen::SchemaSettings::draft07().into_generator();
        serde_json::to_value(generator.into_root_schema_for::<T>()).expect("schema serializes")
    }

    let functions = [
        (
            ON_SESSION_STARTED_FN,
            SESSION_STARTED_DESC,
            serde_json::Value::Null,
            schema::<SessionStartedEvent>(),
        ),
        (
            ON_SESSION_STOPPED_FN,
            SESSION_STOPPED_DESC,
            serde_json::Value::Null,
            schema::<SessionStoppedEvent>(),
        ),
        (
            ON_ACTIVITY_FN,
            ACTIVITY_DESC,
            serde_json::Value::Null,
            schema::<ActivityEvent>(),
        ),
        (
            ON_FRAME_CHANGED_FN,
            ON_FRAME_CHANGED_DESC,
            schema::<FrameArgs>(),
            schema::<FrameChange>(),
        ),
    ];
    let mut failures = Vec::new();
    for (function_id, description, args_format, event) in functions {
        let value = serde_json::json!({
            "function_id": function_id,
            "description": description,
            "private": true,
            "args_format": args_format,
            "event_schema": event,
        });
        let mut pretty = serde_json::to_string_pretty(&value).expect("contract serializes");
        pretty.push('\n');
        let file = format!("sources/{}.json", function_id.replace("::", "."));
        if let Err(msg) = support::check_golden(&file, &pretty) {
            failures.push(msg);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
