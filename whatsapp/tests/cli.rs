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

#[tokio::test]
async fn event_import_is_offline_private_and_isolated_to_the_existing_session() {
    use serde_json::json;
    use whatsapp::{config::Config, sessions::SessionManager};
    let temp = TempDir::new().unwrap();
    let config = Config {
        data_dir: temp.path().join("sessions"),
        ..Config::default()
    };
    let manager = SessionManager::open(config.clone()).unwrap();
    manager.create("test").await.unwrap();
    drop(manager);
    let config_path = temp.path().join("config.yaml");
    std::fs::write(&config_path, serde_yaml::to_string(&config).unwrap()).unwrap();
    let events_path = temp.path().join("events.json");
    let page = json!({"events":[{"session_id":"test","kind":"message.received","data":{
        "info":{"id":"fixture","timestamp_ms":1000,"source":{"chat":"123@g.us","sender":"10@s.whatsapp.net","is_from_me":false}},
        "message":{"conversation":"private fixture text"}
    }}]});
    std::fs::write(&events_path, page.to_string()).unwrap();
    let run = |session: &str| {
        Command::new(env!("CARGO_BIN_EXE_whatsapp"))
            .args(["--local-config", "--config"])
            .arg(&config_path)
            .arg("--import-events")
            .arg(&events_path)
            .args(["--import-session", session])
            .env("III_URL", "ws://127.0.0.1:1")
            .current_dir(temp.path())
            .output()
            .unwrap()
    };
    let output = run("test");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let count: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(count["imported"], 1);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private fixture text"));
    let manager = SessionManager::open(config).unwrap();
    assert_eq!(
        manager
            .inbox("test")
            .unwrap()
            .list_messages("123@g.us", None, 50)
            .unwrap()
            .messages
            .len(),
        1
    );
    assert_eq!(
        manager.list().await[0].status,
        whatsapp::sessions::SessionStatus::Stopped
    );
    let rejected = run("different");
    assert!(!rejected.status.success());
    assert!(!temp.path().join("sessions/different").exists());
}
