//! `--manifest` subprocess contract test (registry publish pipeline).

use std::process::Command;

use serde_json::Value;

#[test]
fn manifest_subcommand_emits_valid_json() {
    let bin = env!("CARGO_BIN_EXE_harness");
    let output = Command::new(bin)
        .arg("--manifest")
        .output()
        .expect("spawn harness --manifest");

    assert!(
        output.status.success(),
        "binary exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout).expect("manifest stdout is utf-8");
    let manifest: Value = serde_json::from_str(&stdout).expect("manifest stdout is valid JSON");

    assert_eq!(manifest["name"], env!("CARGO_PKG_NAME"));
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        manifest["description"]
            .as_str()
            .is_some_and(|d| !d.is_empty()),
        "description must be non-empty"
    );
    assert!(
        manifest["default_config"].is_object(),
        "default_config must be an object"
    );
    assert!(
        !manifest["supported_targets"]
            .as_array()
            .expect("supported_targets must be an array")
            .is_empty(),
        "supported_targets must not be empty"
    );
}

#[test]
fn worker_manifest_uses_the_standalone_queue_worker() {
    let manifest_path = format!("{}/iii.worker.yaml", env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(manifest_path).expect("read iii.worker.yaml");
    let manifest: serde_yaml::Value = serde_yaml::from_str(&source).expect("parse worker manifest");
    let dependencies = manifest["dependencies"]
        .as_mapping()
        .expect("dependencies is a mapping");

    assert_eq!(
        dependencies.get(serde_yaml::Value::String("queue".into())),
        Some(&serde_yaml::Value::String("^0.2.0".into()))
    );
    assert!(!dependencies.contains_key(serde_yaml::Value::String("iii-queue".into())));
}

#[test]
fn worker_manifest_uses_the_standalone_state_worker() {
    let manifest_path = format!("{}/iii.worker.yaml", env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(manifest_path).expect("read iii.worker.yaml");
    let manifest: serde_yaml::Value = serde_yaml::from_str(&source).expect("parse worker manifest");
    let dependencies = manifest["dependencies"]
        .as_mapping()
        .expect("dependencies is a mapping");

    assert_eq!(
        dependencies.get(serde_yaml::Value::String("state".into())),
        Some(&serde_yaml::Value::String("^0.21.3".into()))
    );
    assert!(!dependencies.contains_key(serde_yaml::Value::String("iii-state".into())));
}

#[test]
fn worker_manifest_uses_the_standalone_cron_worker() {
    let manifest_path = format!("{}/iii.worker.yaml", env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(manifest_path).expect("read iii.worker.yaml");
    let manifest: serde_yaml::Value = serde_yaml::from_str(&source).expect("parse worker manifest");
    let dependencies = manifest["dependencies"]
        .as_mapping()
        .expect("dependencies is a mapping");

    assert_eq!(
        dependencies.get(serde_yaml::Value::String("cron".into())),
        Some(&serde_yaml::Value::String("^0.21.0".into()))
    );
    assert!(!dependencies.contains_key(serde_yaml::Value::String("iii-cron".into())));
}
