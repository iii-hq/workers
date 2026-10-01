use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn introspection_is_offline_and_does_not_create_session_storage() {
    let temp = TempDir::new().unwrap();
    for flag in ["--manifest", "--catalog"] {
        let output = Command::new(env!("CARGO_BIN_EXE_whatsapp"))
            .arg(flag)
            .env("III_URL", "ws://127.0.0.1:1")
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(value.is_object() || value.is_array());
    }
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
