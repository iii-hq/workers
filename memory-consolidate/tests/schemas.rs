//! Wire-surface pins: the public catalog stays in lockstep with
//! `register_all`, every function publishes typed request AND response
//! schemas, and ids are kebab-case `memory-consolidate::` names.

use memory_consolidate::functions::catalog;

#[test]
fn catalog_has_all_public_functions() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec!["memory-consolidate::run", "memory-consolidate::status"]
    );
}

#[test]
fn every_function_has_typed_schemas_and_description() {
    for spec in catalog() {
        assert!(
            !spec.description.is_empty(),
            "{} needs a description",
            spec.function_id
        );
        let req = serde_json::to_value(&spec.request_schema).unwrap();
        let resp = serde_json::to_value(&spec.response_schema).unwrap();
        for (which, schema) in [("request", &req), ("response", &resp)] {
            let ty = schema.get("type").and_then(|t| t.as_str());
            assert_eq!(
                ty,
                Some("object"),
                "{} {which} schema must be a typed object, got: {schema}",
                spec.function_id
            );
        }
    }
}

#[test]
fn ids_are_kebab_case_names() {
    for spec in catalog() {
        assert!(
            spec.function_id.starts_with("memory-consolidate::"),
            "{}",
            spec.function_id
        );
        assert!(
            !spec.function_id.contains('_'),
            "{} must be kebab-case",
            spec.function_id
        );
    }
}

mod support;

fn golden_file_name(function_id: &str) -> String {
    format!("schemas/{}.json", function_id.replace("::", "."))
}

fn spec_to_pretty_json(spec: &memory_consolidate::functions::FunctionSpec) -> String {
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

/// Every catalog entry matches its committed golden snapshot — the wire
/// surface consumed by callers and agents. Any schema or description
/// change must land as an explicit golden diff (`UPDATE_GOLDENS=1 cargo
/// test` regenerates).
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

/// A golden file with no catalog entry is a stale leftover.
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
            "orphan golden tests/golden/schemas/{name}: no catalog entry produces it. \
             Delete it or fix the catalog."
        );
    }
}
