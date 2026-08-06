use std::process::Command;

use serde_json::Value;

#[test]
fn manifest_subcommand_emits_valid_json() {
    let bin = env!("CARGO_BIN_EXE_editor");
    let output = Command::new(bin)
        .arg("--manifest")
        .output()
        .expect("spawn editor --manifest");

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
    assert!(!manifest["description"].as_str().unwrap().is_empty());
    assert!(manifest["default_config"].is_object());
    assert!(!manifest["supported_targets"]
        .as_array()
        .expect("supported_targets must be an array")
        .is_empty());
}

/// `--manifest` must not touch the engine: the registry publish pipeline runs
/// it on a host with no iii running, and a connection attempt there would hang
/// the publish rather than fail it.
#[test]
fn manifest_subcommand_needs_no_engine() {
    let bin = env!("CARGO_BIN_EXE_editor");
    let output = Command::new(bin)
        .args(["--manifest", "--url", "ws://127.0.0.1:1"])
        .output()
        .expect("spawn editor --manifest");
    assert!(output.status.success());
}
