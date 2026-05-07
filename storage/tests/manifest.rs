//! Validates the `--manifest` subcommand the registry publish pipeline relies on.
//!
//! Spawns the compiled `storage` binary with `--manifest`, parses stdout as
//! JSON, and asserts the five fields required by `POST /publish`:
//! `name`, `version`, `description`, `default_config` (object), and
//! `supported_targets` (non-empty array).

use std::process::Command;

use serde_json::Value;

#[test]
fn manifest_subcommand_emits_valid_json() {
    let bin = env!("CARGO_BIN_EXE_storage");
    let output = Command::new(bin)
        .arg("--manifest")
        .output()
        .expect("spawn storage --manifest");

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
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "description must be a non-empty string"
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
