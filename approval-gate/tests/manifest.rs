use std::process::Command;

use serde_json::Value;

fn binary_path() -> String {
    if let Some(path) = option_env!("CARGO_BIN_EXE_approval_gate") {
        return path.to_string();
    }
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    format!("{manifest_dir}/target/debug/iii-approval-gate")
}

#[test]
fn manifest_subcommand_emits_valid_json() {
    let bin = binary_path();
    let output = Command::new(&bin)
        .arg("--manifest")
        .output()
        .unwrap_or_else(|e| panic!("spawn {bin}: {e}"));

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
            .is_some_and(|s| !s.is_empty()),
        "description must be non-empty"
    );
    assert!(manifest["default_config"].is_object());
    assert!(
        !manifest["supported_targets"]
            .as_array()
            .expect("supported_targets must be an array")
            .is_empty(),
        "supported_targets must not be empty"
    );
}
