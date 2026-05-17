//! Smoke test for the `--manifest` CLI flag. Mirrors `policy-denylist`'s
//! approach: spawn the built binary, parse stdout as JSON, assert the
//! shape matches what the harness manifest generator and registry consume.

use std::process::Command;

use serde_json::Value;

fn binary_path() -> String {
    // Prefer the Cargo-injected path when available (standard cargo test
    // builds). Fall back to constructing it from CARGO_MANIFEST_DIR so the
    // test still works in environments where CARGO_BIN_EXE_* is not
    // injected (e.g. some sandboxed CI runners).
    if let Some(path) = option_env!("CARGO_BIN_EXE_context-compaction") {
        return path.to_string();
    }
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    format!("{manifest_dir}/target/debug/context-compaction")
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

    assert_eq!(manifest["name"], "context-compaction");
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        manifest["description"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "description must be non-empty"
    );

    // The compactor is not LLM-facing: it must not register any callable
    // function. Regressing this would expose `result::fetch`-style surface
    // the model could discover via `engine::functions::list`.
    let functions = manifest["functions"]
        .as_array()
        .expect("functions must be an array");
    assert!(
        functions.is_empty(),
        "compactor must not register LLM-facing functions, got {functions:?}"
    );

    // The whole worker's purpose is the agent::events subscription; if this
    // ever drops out of the manifest the harness wiring stops registering
    // the trigger and compaction silently never fires.
    let subscriptions = manifest["subscriptions"]
        .as_array()
        .expect("subscriptions must be an array");
    assert!(
        subscriptions
            .iter()
            .any(|s| s.as_str() == Some("agent::events")),
        "subscriptions must include agent::events, got {subscriptions:?}"
    );

    assert_eq!(manifest["skill_id"], "context-compaction");
    assert!(
        manifest["skill"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "skill body must be embedded"
    );
}
