//! Integration test: spawn the `iii-introspect` binary with `--manifest`
//! and validate the emitted JSON manifest. This guards the contract the
//! registry publish pipeline depends on without requiring a live engine.

use std::process::Command;

use serde_json::Value;

#[test]
fn manifest_subcommand_emits_valid_json() {
    let bin = env!("CARGO_BIN_EXE_iii-introspect");
    let output = Command::new(bin)
        .arg("--manifest")
        .output()
        .expect("spawn iii-introspect --manifest");

    assert!(
        output.status.success(),
        "binary exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout).expect("manifest stdout is utf-8");
    let manifest: Value = serde_json::from_str(&stdout).expect("manifest stdout is valid JSON");

    assert_eq!(manifest["name"], "iii-introspect");
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        manifest["description"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "description should be a non-empty string"
    );

    let defaults = &manifest["default_config"];
    assert!(defaults.is_object(), "default_config must be an object");
    assert!(
        defaults["class"].as_str().is_some_and(|s| !s.is_empty()),
        "default_config.class should be set"
    );
    assert_eq!(defaults["config"]["cron_topology_refresh"], "0 */5 * * * *");
    assert_eq!(defaults["config"]["cache_ttl_seconds"], 30);

    let targets = manifest["supported_targets"]
        .as_array()
        .expect("supported_targets must be an array");
    assert!(!targets.is_empty(), "supported_targets must not be empty");
}
