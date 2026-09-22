//! Catalog-level checks: the function ids the permissions files reference
//! exist, and the configuration schema accepts its own defaults.

use iii_stories::config::{StoriesConfig, normalize, schema};
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
    ] {
        assert!(
            permissions.contains(internal),
            "{internal} missing from iii-permissions.yaml"
        );
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
