use std::process::Command;

fn manifest_json() -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_tailscale"))
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

#[test]
fn manifest_subcommand_emits_the_publish_fields() {
    let manifest = manifest_json();
    assert_eq!(manifest["name"], "tailscale");
    for key in [
        "version",
        "description",
        "default_config",
        "supported_targets",
    ] {
        assert!(!manifest[key].is_null(), "manifest is missing {key}");
    }
    assert_eq!(manifest["default_config"]["allow_funnel"], false);
    assert_eq!(
        manifest["default_config"]["console_url"],
        "http://127.0.0.1:3113"
    );
}
