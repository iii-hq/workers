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
fn worker_manifest_declares_runtime_dependencies() {
    let source = std::fs::read_to_string(format!("{}/iii.worker.yaml", env!("CARGO_MANIFEST_DIR")))
        .expect("read iii.worker.yaml");
    let manifest: serde_yaml::Value = serde_yaml::from_str(&source).expect("parse worker manifest");
    let dependencies = manifest["dependencies"]
        .as_mapping()
        .expect("dependencies map");
    for dependency in ["state", "queue", "cron", "observability"] {
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
/// configuration), which `iii worker add` rejects: its resolver caps a
/// graph at five. eval still calls `harness::*` at run time, so the
/// harness worker must be present in the engine — it is simply not pulled
/// in by eval's own install.
#[test]
fn worker_manifest_omits_harness_to_stay_within_install_graph_depth() {
    let source = std::fs::read_to_string(format!("{}/iii.worker.yaml", env!("CARGO_MANIFEST_DIR")))
        .expect("read iii.worker.yaml");
    let manifest: serde_yaml::Value = serde_yaml::from_str(&source).expect("parse worker manifest");
    let dependencies = manifest["dependencies"]
        .as_mapping()
        .expect("dependencies map");
    assert!(
        !dependencies.contains_key(serde_yaml::Value::String("harness".into())),
        "harness must stay out of the manifest: it pushes the install graph past the depth cap"
    );
}
