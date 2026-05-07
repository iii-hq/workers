//! `iii-provider-openai --manifest` JSON contract (registry publish).

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn binary_path() -> PathBuf {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    #[cfg(target_os = "windows")]
    let name = "iii-provider-openai.exe";
    #[cfg(not(target_os = "windows"))]
    let name = "iii-provider-openai";
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(name)
}

#[test]
fn manifest_subcommand_emits_valid_json() {
    let bin = binary_path();
    let output = Command::new(&bin)
        .arg("--manifest")
        .output()
        .unwrap_or_else(|e| panic!("spawn {:?}: {e}", bin));

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
