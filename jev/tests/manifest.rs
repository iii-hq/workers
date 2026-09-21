use serde_json::Value;
use std::process::Command;
#[test]
fn manifest_is_complete_without_engine_key_or_seed() {
    let output = Command::new(env!("CARGO_BIN_EXE_jev"))
        .args([
            "--manifest",
            "--url",
            "ws://127.0.0.1:1",
            "--config",
            "/missing/seed.yaml",
        ])
        .env_remove("TYPESAFE_API_KEY")
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["name"], "jev");
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert!(value["description"].as_str().is_some_and(|s| s.len() > 20));
    assert_eq!(
        value["default_config"],
        jev::config::JevConfig::default().to_json()
    );
    assert!(value["supported_targets"][0]
        .as_str()
        .is_some_and(|s| s.contains('-')));
}
