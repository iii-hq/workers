use serde_json::Value;
use std::collections::BTreeSet;

fn typed(schema: &Value) -> bool {
    [
        "type",
        "properties",
        "$ref",
        "allOf",
        "anyOf",
        "oneOf",
        "enum",
        "items",
        "const",
    ]
    .iter()
    .any(|key| schema.get(key).is_some())
}

#[test]
fn every_function_has_a_unique_id_and_typed_schemas() {
    let catalog = whatsapp::registry().catalog();
    let mut ids = BTreeSet::new();
    for function in &catalog {
        assert!(function.id.starts_with("whatsapp::"));
        assert!(ids.insert(function.id.clone()), "duplicate {}", function.id);
        assert!(!function.description.trim().is_empty());
        assert!(
            typed(&function.request_format),
            "untyped request: {}",
            function.id
        );
        assert!(
            typed(&function.response_format),
            "untyped response: {}",
            function.id
        );
    }
    for required in [
        "whatsapp::sessions::create",
        "whatsapp::sessions::start",
        "whatsapp::sessions::stop",
        "whatsapp::sessions::qr",
        "whatsapp::messages::send_text",
        "whatsapp::media::send",
        "whatsapp::groups::create",
        "whatsapp::events::poll",
        "whatsapp::calls::reject",
    ] {
        assert!(ids.contains(required), "missing {required}");
    }
}

#[test]
fn coverage_accounts_for_every_openwa_operation_and_only_real_functions() {
    let coverage: Value = serde_json::from_str(include_str!("../openwa-coverage.json")).unwrap();
    let operations = coverage["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 202);
    let catalog: BTreeSet<_> = whatsapp::registry()
        .catalog()
        .into_iter()
        .map(|item| item.id)
        .collect();
    let mut operation_ids = BTreeSet::new();
    let mut routes = BTreeSet::new();
    for operation in operations {
        let id = operation["operationId"].as_str().unwrap();
        assert!(operation_ids.insert(id));
        let route = (
            operation["method"].as_str().unwrap(),
            operation["path"].as_str().unwrap(),
        );
        assert!(routes.insert(route));
        let status = operation["status"].as_str().unwrap();
        assert!(["native_equivalent", "partial", "not_implemented"].contains(&status));
        let functions = operation["functions"].as_array().unwrap();
        if status == "native_equivalent" {
            assert!(!functions.is_empty(), "{id} has no mapping");
        }
        for function in functions {
            let function = function.as_str().unwrap();
            assert!(
                catalog.contains(function),
                "{id} maps to nonexistent {function}"
            );
        }
        assert!(!operation["notes"].as_str().unwrap().is_empty());
    }
}
