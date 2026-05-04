//! `--manifest` smoke test. Same shape as the sibling workers — runs in
//! CI without an iii engine, and the registry publish pipeline depends
//! on this exact contract.

use std::process::Command;

use serde_json::Value;

#[test]
fn manifest_subcommand_emits_valid_json() {
    let bin = env!("CARGO_BIN_EXE_mcp");
    let output = Command::new(bin)
        .arg("--manifest")
        .output()
        .expect("spawn mcp --manifest");

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
    assert!(manifest["description"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));
    assert_eq!(manifest["default_config"]["api_path"], "mcp");
    assert_eq!(manifest["default_config"]["state_timeout_ms"], 30_000);
    assert!(manifest["default_config"]["hidden_prefixes"]
        .as_array()
        .is_some_and(|a| !a.is_empty()));
    assert!(!manifest["supported_targets"]
        .as_array()
        .expect("supported_targets must be an array")
        .is_empty());
}
