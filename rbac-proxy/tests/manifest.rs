//! Validates the `--manifest` subcommand the registry publish pipeline
//! depends on. Spawns the binary, parses stdout as JSON, asserts the required
//! fields are present and shaped correctly.

use std::process::Command;

use serde_json::Value;

#[test]
fn manifest_subcommand_emits_valid_json() {
    let bin = env!("CARGO_BIN_EXE_rbac-proxy");
    let output = Command::new(bin)
        .arg("--manifest")
        .output()
        .expect("spawn rbac-proxy --manifest");

    assert!(
        output.status.success(),
        "binary exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout).expect("manifest stdout is utf-8");
    let manifest: Value = serde_json::from_str(&stdout).expect("manifest stdout is valid JSON");

    assert_eq!(manifest["name"], "rbac-proxy");
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        manifest["description"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "description must be a non-empty string"
    );
    assert!(
        manifest["default_config"].is_object(),
        "default_config must be an object"
    );
    let cfg = manifest["default_config"].as_object().unwrap();
    for key in ["host", "port", "engine_url", "expose_worker_internals", "rbac"] {
        assert!(cfg.contains_key(key), "default_config.{key} missing");
    }
    assert_eq!(cfg["port"], 49200, "default public RBAC port");
    assert!(
        !manifest["supported_targets"]
            .as_array()
            .expect("supported_targets must be an array")
            .is_empty(),
        "supported_targets must not be empty"
    );
}
