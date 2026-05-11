use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn provider_router_executable() -> PathBuf {
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_provider_router") {
        return PathBuf::from(p);
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for profile in ["debug", "release"] {
        let candidate = dir.join("target").join(profile).join("provider-router");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "iii-provider-router binary not found under target/{{debug,release}}/; run `cargo build --bin iii-provider-router` first"
    );
}

#[test]
fn manifest_subcommand_emits_valid_json() {
    let bin = provider_router_executable();
    let output = Command::new(&bin)
        .arg("--manifest")
        .output()
        .expect("spawn provider-router --manifest");

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
