//! The published `ios-simulator::*` surface: ids, descriptions and schemas.

use std::collections::HashSet;

use ios_simulator::functions::catalog;

#[test]
fn ids_are_unique_and_namespaced() {
    let specs = catalog();
    let ids: HashSet<&str> = specs.iter().map(|s| s.id).collect();
    assert_eq!(ids.len(), specs.len(), "duplicate function id");
    for spec in &specs {
        assert!(spec.id.starts_with("ios-simulator::"), "{}", spec.id);
        assert!(
            !spec.description.is_empty(),
            "{} has no description",
            spec.id
        );
    }
}

#[test]
fn every_call_takes_an_optional_tenant() {
    for spec in catalog() {
        let schema = serde_json::to_value(&spec.request).unwrap();
        assert!(
            schema["properties"]["tenant"].is_object(),
            "{} does not accept tenant",
            spec.id
        );
        let required: Vec<&str> = schema["required"]
            .as_array()
            .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(!required.contains(&"tenant"), "{} requires tenant", spec.id);
    }
}

#[test]
fn device_calls_require_a_udid() {
    let needs_udid = [
        "ios-simulator::devices::boot",
        "ios-simulator::open",
        "ios-simulator::devices::shutdown",
        "ios-simulator::devices::erase",
        "ios-simulator::devices::delete",
        "ios-simulator::screenshot",
        "ios-simulator::recording::start",
        "ios-simulator::recording::stop",
        "ios-simulator::gesture",
        "ios-simulator::button",
        "ios-simulator::type",
        "ios-simulator::key",
        "ios-simulator::touch",
        "ios-simulator::watch",
        "ios-simulator::frame",
    ];
    let specs = catalog();
    for id in needs_udid {
        let spec = specs
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("{id} missing"));
        let schema = serde_json::to_value(&spec.request).unwrap();
        let required = schema["required"].as_array().cloned().unwrap_or_default();
        assert!(
            required.contains(&serde_json::json!("udid")),
            "{id} must require udid"
        );
    }
}

#[test]
fn ui_plumbing_is_marked_internal() {
    for spec in catalog() {
        if matches!(
            spec.id,
            "ios-simulator::touch" | "ios-simulator::watch" | "ios-simulator::frame"
        ) {
            assert!(spec.description.starts_with("Internal:"), "{}", spec.id);
        }
    }
}
