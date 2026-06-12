//! @pure steps for the `--manifest` registry contract: spawn the real
//! binary and validate its JSON output.

use cucumber::{then, when};
use serde_json::Value;

use crate::common::world::SessionWorld;

#[when("I run the worker binary with --manifest")]
async fn run_manifest(world: &mut SessionWorld) {
    let bin = env!("CARGO_BIN_EXE_session-manager");
    let output = std::process::Command::new(bin)
        .arg("--manifest")
        .output()
        .expect("spawn session-manager --manifest");
    assert!(
        output.status.success(),
        "binary exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("manifest stdout is utf-8");
    let manifest: Value = serde_json::from_str(&stdout).expect("manifest stdout is valid JSON");
    world.last_response = Some(manifest);
    world.last_error = None;
}

#[then("the manifest has every required registry field")]
async fn manifest_required_fields(world: &mut SessionWorld) {
    let manifest = world.last_response.as_ref().expect("run --manifest first");
    assert_eq!(manifest["name"], "session-manager");
    assert!(
        manifest["version"].as_str().is_some_and(|v| !v.is_empty()),
        "version must be a non-empty string"
    );
    assert!(
        manifest["description"]
            .as_str()
            .is_some_and(|d| !d.is_empty()),
        "description must be a non-empty string"
    );
    assert!(
        manifest["default_config"].is_object(),
        "default_config must be an object"
    );
    assert!(
        manifest["supported_targets"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "supported_targets must be a non-empty array"
    );
}
