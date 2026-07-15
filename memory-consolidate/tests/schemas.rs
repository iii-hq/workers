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
