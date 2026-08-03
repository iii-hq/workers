//! The `--manifest` subcommand must emit valid module-manifest JSON: cargo
//! builds the binary for this test and hands us its path via
//! `CARGO_BIN_EXE_computer`.

use std::process::Command;

#[test]
fn manifest_subcommand_emits_valid_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_computer"))
        .arg("--manifest")
        .output()
        .expect("run computer --manifest");
    assert!(
        output.status.success(),
        "--manifest exited with {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("manifest stdout is valid JSON");
    assert_eq!(parsed["name"], "computer");
    assert!(parsed["version"].is_string());
    assert!(parsed["default_config"].is_object());
    assert!(!parsed["supported_targets"]
        .as_array()
        .expect("supported_targets is an array")
        .is_empty());
}
