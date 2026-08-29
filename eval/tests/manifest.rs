use std::process::Command;

use serde_json::Value;

#[test]
fn manifest_subcommand_emits_valid_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_eval"))
        .arg("--manifest")
        .output()
        .expect("spawn eval --manifest");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: Value = serde_json::from_slice(&output.stdout).expect("valid manifest JSON");
    assert_eq!(manifest["name"], "eval");
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    assert!(manifest["description"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert!(manifest["default_config"].is_object());
    assert!(manifest["supported_targets"]
        .as_array()
        .is_some_and(|targets| !targets.is_empty()));
}

#[test]
fn worker_catalog_declares_runtime_dependencies() {
    let catalog: serde_yaml::Value = serde_yaml::from_str(include_str!("../../worker-compose.yaml")).expect("parse worker catalog");
    let dependencies = catalog["workers"]["eval"]["registry"]["dependencies"]
        .as_mapping()
        .expect("dependencies map");
    for dependency in ["state", "queue", "cron", "iii-observability"] {
        assert!(
            dependencies.contains_key(serde_yaml::Value::String(dependency.into())),
            "missing {dependency}"
        );
    }
}

/// `harness` is deliberately NOT a declared dependency.
///
/// Declaring it made `eval`'s resolved install graph six levels deep
/// (eval -> harness -> context-manager -> llm-router -> state ->
/// configuration), which the former worker installer rejected because its
/// resolver capped a graph at five. eval still calls `harness::*` at run time, so the
/// harness worker must be present in the engine — it is simply not pulled
/// in by eval's own install.
#[test]
fn worker_catalog_omits_harness_to_stay_within_install_graph_depth() {
    let catalog: serde_yaml::Value = serde_yaml::from_str(include_str!("../../worker-compose.yaml")).expect("parse worker catalog");
    let dependencies = catalog["workers"]["eval"]["registry"]["dependencies"]
        .as_mapping()
        .expect("dependencies map");
    assert!(
        !dependencies.contains_key(serde_yaml::Value::String("harness".into())),
        "harness must stay out of the catalog entry: it pushes the install graph past the depth cap"
    );
}
