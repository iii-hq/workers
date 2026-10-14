use std::process::Command;

fn manifest_json() -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_tmux"))
        .arg("--manifest")
        .output()
        .expect("the worker binary runs");
    assert!(
        output.status.success(),
        "--manifest exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("--manifest prints JSON")
}

#[test]
fn manifest_subcommand_emits_the_publish_fields() {
    let manifest = manifest_json();
    assert_eq!(manifest["name"], "tmux");
    for key in [
        "version",
        "description",
        "default_config",
        "supported_targets",
    ] {
        assert!(manifest.get(key).is_some(), "manifest is missing {key}");
    }
    assert_eq!(manifest["default_config"]["tmux_binary"], "tmux");
}

#[test]
fn the_binary_never_answers_a_tmux_probe() {
    // tmux resolution probes candidates with `-V` and this env var set; the
    // worker binary, which is also called `tmux`, must not pass for tmux.
    let output = Command::new(env!("CARGO_BIN_EXE_tmux"))
        .arg("-V")
        .env(tmux::binary::PROBE_ENV, "1")
        .output()
        .expect("the worker binary runs");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
