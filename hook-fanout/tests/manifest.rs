use std::process::Command;

use serde_json::Value;

fn binary_path() -> String {
    #[allow(clippy::option_env_unwrap)]
    if let Some(path) = option_env!("CARGO_BIN_EXE_hook_fanout") {
        return path.to_string();
    }
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    format!("{manifest_dir}/target/debug/hook-fanout")
}

#[test]
fn manifest_subcommand_emits_valid_json() {
    let output = Command::new(binary_path())
        .arg("--manifest")
        .output()
        .expect("spawn hook-fanout --manifest");

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
    assert!(manifest["default_config"].is_object());
    assert!(!manifest["supported_targets"]
        .as_array()
        .expect("supported_targets must be an array")
        .is_empty());
}
