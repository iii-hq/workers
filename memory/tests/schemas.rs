//! Wire-surface pins: every registered function publishes typed request
//! AND response schemas (no `Value` handlers), ids are kebab-case
//! `memory::` names, and the catalog stays in lockstep with
//! `register_all`.

use memory::functions::catalog;

#[test]
fn catalog_has_all_public_functions() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "memory::bank::create",
            "memory::bank::list",
            "memory::bank::delete",
            "memory::save",
            "memory::get",
            "memory::list",
            "memory::update",
            "memory::delete",
            "memory::pin",
            "memory::supersede",
            "memory::recall",
            "memory::rule::list",
            "memory::rule::set",
            "memory::doctor",
            "memory::reload",
        ]
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
fn ids_are_kebab_case_memory_names() {
    for spec in catalog() {
        assert!(
            spec.function_id.starts_with("memory::"),
            "{}",
            spec.function_id
        );
        assert!(
            !spec.function_id.contains('_'),
            "{} must use kebab-case segments",
            spec.function_id
        );
    }
}

#[test]
fn recall_response_reports_retrieval_mode() {
    // The degraded-state honesty contract: recall always says which
    // retrieval mode ran.
    let spec = catalog()
        .into_iter()
        .find(|s| s.function_id == "memory::recall")
        .unwrap();
    let resp = serde_json::to_value(&spec.response_schema).unwrap();
    let props = resp["properties"].as_object().unwrap();
    assert!(props.contains_key("retrieval"));
    assert!(props.contains_key("memories"));
    assert!(props.contains_key("bank"));
}
