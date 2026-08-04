//! The `--manifest` contract, exercised through the real binary.
//!
//! The unit tests in `src/manifest.rs` cover the struct; this covers the CLI
//! path the registry publish pipeline actually calls, including the part that
//! matters most: it must print and exit without touching the engine.

use std::process::Command;

fn manifest_json() -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_pdf"))
        .arg("--manifest")
        .output()
        .expect("the worker binary runs");

    assert!(
        output.status.success(),
        "--manifest exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "--manifest did not print JSON ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// `POST /publish` rejects a manifest missing any of these five.
#[test]
fn manifest_prints_every_field_the_registry_requires() {
    let json = manifest_json();

    assert_eq!(json["name"], "pdf");
    assert!(
        json["version"].as_str().is_some_and(|v| !v.is_empty()),
        "version missing"
    );
    assert!(
        json["description"].as_str().is_some_and(|d| d.len() > 20),
        "description missing or too short to be useful in the registry"
    );
    assert!(json["default_config"].is_object(), "default_config missing");
    assert!(
        json["supported_targets"]
            .as_array()
            .is_some_and(|t| !t.is_empty()),
        "supported_targets missing or empty"
    );
}

/// The build script forwards the build-time triple. A manifest advertising a
/// target the binary was not built for would hand consumers the wrong artifact.
#[test]
fn supported_targets_carries_a_real_triple() {
    let json = manifest_json();
    let target = json["supported_targets"][0]
        .as_str()
        .expect("a target triple");
    assert!(
        target.contains('-'),
        "{target} does not look like a target triple"
    );
}

/// The manifest path must not need an engine: the publish pipeline runs it on a
/// bare runner with nothing listening.
#[test]
fn manifest_needs_no_engine() {
    let output = Command::new(env!("CARGO_BIN_EXE_pdf"))
        .arg("--manifest")
        // A URL nothing is listening on. Connecting would hang or fail; the
        // manifest path must return before it ever tries.
        .args(["--url", "ws://127.0.0.1:1"])
        .output()
        .expect("the worker binary runs");

    assert!(
        output.status.success(),
        "--manifest must not depend on an engine, got {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The default config the registry publishes has to be the config the worker
/// actually boots with, or an operator reading the registry is misled.
#[test]
fn published_default_config_matches_the_shipped_defaults() {
    let json = manifest_json();
    assert_eq!(
        json["default_config"],
        pdf::config::WorkerConfig::default().to_json()
    );
}
