use std::process::Command;

#[test]
fn manifest_command_outputs_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_iii-auth"))
        .arg("--manifest")
        .output()
        .expect("run manifest command");
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["name"], "auth");
    assert!(json["default_config"].is_object());
}
