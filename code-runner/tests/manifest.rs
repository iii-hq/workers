//! The binary's `--manifest` output: valid JSON with the registry fields,
//! printed without connecting to anything.

#[test]
fn manifest_flag_prints_valid_json_and_exits_zero() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_code-runner"))
        .arg("--manifest")
        .output()
        .expect("binary runs");
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout is valid JSON");
    assert_eq!(parsed["name"], "code-runner");
    assert!(parsed["default_config"].is_object());
    assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
}
