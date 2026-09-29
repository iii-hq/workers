//! Wire-schema snapshots for the agent-facing `harness::*` functions.
//!
//! `harness::surface::catalog()` is the single source of truth for each
//! function's id and schemars-derived request/response schemas (generated with
//! the same `SchemaSettings::draft07()` construction iii-sdk uses at
//! registration). Each entry is serialized to pretty JSON and compared against
//! `tests/golden/schemas/<id>.json` (`::` maps to `.` in filenames).
//!
//! Regenerate with `UPDATE_GOLDENS=1 cargo test`.

mod support;

use harness::surface::{catalog, FunctionSpec};

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

#[test]
fn deletion_trigger_schema_matches_golden() {
    let actual = serde_json::json!({
        "trigger_type": harness::deletion_events::TYPE,
        "trigger_request_format": harness::surface::schema_value::<harness::deletion_events::DeletionConfig>(),
        "call_request_format": harness::surface::schema_value::<harness::functions::delete_session_tree::Snapshot>(),

    });
    support::check_golden(
        "schemas/harness.session-tree-deletion.json",
        &(serde_json::to_string_pretty(&actual).unwrap() + "\n"),
    )
    .unwrap();
}

#[test]
fn catalog_lists_all_functions_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "harness::send",
            "harness::spawn",
            "harness::turn",
            "harness::function::trigger",
            "harness::function::resolve",
            "harness::stop",
            "harness::delete-session-tree",
            "harness::delete-session-tree-status",
            "harness::status",
            "harness::system-prompt::get",
            "harness::session-tree",
            "harness::metrics",
            "harness::triggers::list",
            "harness::triggers::unregister",
        ]
    );
}

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

#[test]
fn metrics_request_schema_accepts_exactly_one_session_id_spelling() {
    use harness::functions::metrics::SessionMetricsRequestV1;
    use serde_json::json;

    let spec = catalog()
        .into_iter()
        .find(|spec| spec.function_id == "harness::metrics")
        .unwrap();
    let schema = serde_json::to_value(spec.request_schema).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    for (payload, valid) in [
        (json!({"root_session_id": "s_root"}), true),
        (json!({"session_id": "s_child"}), true),
        (json!({"session_id": "s_child", "extra": true}), true),
        (json!({}), false),
        (
            json!({"root_session_id": "s_root", "session_id": "s_root"}),
            false,
        ),
        (
            json!({"root_session_id": "s_root", "session_id": "s_child"}),
            false,
        ),
        (json!({"root_session_id": null}), false),
        (json!({"session_id": null}), false),
        (json!({"root_session_id": 42}), false),
        (json!({"session_id": 42}), false),
    ] {
        assert_eq!(
            validator.is_valid(&payload),
            valid,
            "schema validation for {payload}"
        );
        assert_eq!(
            serde_json::from_value::<SessionMetricsRequestV1>(payload.clone()).is_ok(),
            valid,
            "deserialization for {payload}"
        );
    }
}

#[test]
fn metrics_response_schema_requires_counters_but_accepts_null_and_measured_values() {
    use harness::functions::metrics::SessionMetricsResponseV1;
    use serde_json::json;

    let spec = catalog()
        .into_iter()
        .find(|spec| spec.function_id == "harness::metrics")
        .unwrap();
    let schema = serde_json::to_value(spec.response_schema).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let counters = [
        "input_tokens",
        "output_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
        "reasoning_tokens",
        "cost_usd",
    ];
    for value in [json!(null), json!(0), json!(7)] {
        let mut response = json!({
            "root_session_id": "s_root",
            "complete": true,
            "totals": {
                "sessions": 1,
                "turns": 1,
                "function_calls": 0,
                "function_call_errors": 0
            },
            "by_session": [{
                "session_id": "s_root",
                "depth": 0,
                "turns": 1,
                "function_calls": 0,
                "function_call_errors": 0
            }]
        });
        for counter in counters {
            response["totals"][counter] = value.clone();
            response["by_session"][0][counter] = value.clone();
        }
        let response = serde_json::to_value(
            serde_json::from_value::<SessionMetricsResponseV1>(response).unwrap(),
        )
        .unwrap();
        assert!(
            validator.is_valid(&response),
            "response with {value} counters"
        );
        for object in ["/totals", "/by_session/0"] {
            for counter in counters {
                let mut missing = response.clone();
                missing
                    .pointer_mut(object)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(counter);
                assert!(
                    !validator.is_valid(&missing),
                    "{object}/{counter} must be present even when null"
                );
            }
        }
    }
}

#[test]
fn deletion_snapshot_schema_rejects_zero_attempt() {
    use harness::functions::delete_session_tree::Snapshot;
    use serde_json::json;

    let spec = catalog()
        .into_iter()
        .find(|spec| spec.function_id == "harness::delete-session-tree")
        .unwrap();
    let schema = serde_json::to_value(spec.response_schema).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let valid = json!({"operation_id":"op", "attempt":1, "session_id":"s", "status":"deleting", "deleted_session_ids":[]});
    let invalid = json!({"operation_id":"op", "attempt":0, "session_id":"s", "status":"deleting", "deleted_session_ids":[]});
    assert!(validator.is_valid(&valid));
    assert!(!validator.is_valid(&invalid));
    assert!(serde_json::from_value::<Snapshot>(invalid).is_err());
}
