//! Catalog-level checks: the function ids the permissions files reference
//! exist, and the configuration schema accepts its own defaults.

use iii_stories::config::{StoriesConfig, normalize, schema};
use iii_stories::events::{
    BUILD_TRIGGER, CHANGED_TRIGGER, ON_BUILD_FN, ON_CHANGED_FN, refuse_build, refuse_changed,
};
use iii_stories::functions::FUNCTION_IDS;

#[test]
fn permissions_file_names_every_public_function() {
    let permissions =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/iii-permissions.yaml"))
            .unwrap();
    for id in FUNCTION_IDS {
        assert!(
            permissions.contains(id),
            "{id} missing from iii-permissions.yaml"
        );
    }
    for internal in [
        "!stories::ui-content",
        "!stories::ui-file",
        "!stories::on-config-change",
        "!stories::on-changed",
        "!stories::on-build",
    ] {
        assert!(
            permissions.contains(internal),
            "{internal} missing from iii-permissions.yaml"
        );
    }
}

#[test]
fn observable_functions_sit_next_to_their_trigger_types_and_are_not_invocable() {
    assert_eq!(CHANGED_TRIGGER, "stories:changed");
    assert_eq!(BUILD_TRIGGER, "stories:build");
    assert_eq!(ON_CHANGED_FN, "stories::on-changed");
    assert_eq!(ON_BUILD_FN, "stories::on-build");
    for id in [ON_CHANGED_FN, ON_BUILD_FN] {
        assert!(
            !FUNCTION_IDS.contains(&id),
            "{id} is observed, never called: it is not a public function"
        );
    }
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/iii.worker.yaml")).unwrap();
    for id in [ON_CHANGED_FN, ON_BUILD_FN] {
        assert!(readme.contains(id), "{id} missing from README.md");
        assert!(manifest.contains(id), "{id} missing from iii.worker.yaml");
    }
}

#[test]
fn each_on_function_refuses_direct_invocation() {
    for (id, refusal) in [
        (ON_CHANGED_FN, refuse_changed(serde_json::json!({}))),
        (
            ON_BUILD_FN,
            refuse_build(serde_json::json!({ "workspace": "ws" })),
        ),
    ] {
        match refusal {
            Err(iii_sdk::errors::Error::Remote { code, message, .. }) => {
                assert_eq!(code, iii_sdk::protocol::observe_codes::FORBIDDEN);
                assert!(message.contains(id), "{message}");
            }
            other => panic!("{id}: expected FORBIDDEN, got {other:?}"),
        }
    }
}

#[test]
fn defaults_satisfy_the_schema_shape() {
    let defaults = StoriesConfig::default();
    let schema = schema();
    let properties = schema["properties"].as_object().unwrap();
    let json = defaults.to_json();
    for key in json.as_object().unwrap().keys() {
        assert!(
            properties.contains_key(key),
            "default field {key} is not in the schema"
        );
    }
    assert_eq!(normalize(&json), defaults);
}

#[test]
fn worker_manifest_declares_the_binary_and_dependencies() {
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/iii.worker.yaml")).unwrap();
    assert!(manifest.contains("name: stories"));
    assert!(manifest.contains("bin: stories"));
    assert!(manifest.contains("browser:"));
    assert!(manifest.contains("configuration:"));
}
