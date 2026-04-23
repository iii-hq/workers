//! Integration test: spawn the `iii-experiment` binary with `--manifest`
//! and validate the emitted JSON manifest. This exercises the same code path
//! the registry publish pipeline relies on, without booting the WebSocket
//! runtime or talking to the III engine.

use std::process::Command;

use serde_json::Value;

#[test]
fn manifest_subcommand_emits_valid_json() {
    let bin = env!("CARGO_BIN_EXE_iii-experiment");
    let output = Command::new(bin)
        .arg("--manifest")
        .output()
        .expect("spawn iii-experiment --manifest");

    assert!(
        output.status.success(),
        "binary exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout).expect("manifest stdout is utf-8");
    let manifest: Value = serde_json::from_str(&stdout).expect("manifest stdout is valid JSON");

    assert_eq!(manifest["name"], "iii-experiment");
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        manifest["description"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "description should be a non-empty string"
    );

    let defaults = &manifest["default_config"];
    assert!(defaults.is_object(), "default_config must be an object");
    assert_eq!(defaults["default_budget"], 20);
    assert_eq!(defaults["max_budget"], 100);
    assert_eq!(defaults["timeout_per_run_ms"], 30000);

    let targets = manifest["supported_targets"]
        .as_array()
        .expect("supported_targets must be an array");
    assert!(!targets.is_empty(), "supported_targets must not be empty");
}
