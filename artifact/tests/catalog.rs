//! The public wire surface: ids, typed schemas and the internal hook staying
//! off the catalog.

use artifact::functions::catalog;

#[test]
fn every_function_is_namespaced_kebab_case_and_typed() {
    for spec in catalog() {
        let id = spec.function_id;
        assert!(id.starts_with("artifact::"), "{id}");
        assert!(!id.contains('_'), "{id} uses snake_case");
        assert_eq!(id.to_lowercase(), id);

        let request = serde_json::to_value(&spec.request_schema).expect("request schema");
        let response = serde_json::to_value(&spec.response_schema).expect("response schema");
        assert_eq!(request["type"], "object", "{id} request is not an object");
        assert_eq!(response["type"], "object", "{id} response is not an object");
    }
}

#[test]
fn the_config_reload_hook_is_not_public() {
    assert!(
        catalog()
            .iter()
            .all(|spec| spec.function_id != "artifact::on-config-change")
    );
}

#[test]
fn share_requires_only_the_artifact_id() {
    let share = catalog()
        .into_iter()
        .find(|spec| spec.function_id == "artifact::share")
        .expect("share is registered");
    let schema = serde_json::to_value(&share.request_schema).expect("schema");
    assert_eq!(schema["required"], serde_json::json!(["id"]));
}
