//! Explicit offline engine integration: III_BINARY=/path/to/iii cargo test --test engine_smoke -- --ignored
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    net::TcpListener,
    path::Path,
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn trigger(binary: &Path, port: u16, id: &str, payload: Value) -> Result<Value, String> {
    let output = Command::new(binary)
        .args([
            "trigger",
            "--port",
            &port.to_string(),
            id,
            "--json",
            &payload.to_string(),
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    // The iii CLI emits no stdout for a successful null response.
    if output.stdout.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("{e}: {}", String::from_utf8_lossy(&output.stdout)))
}

fn wait_for(mut probe: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !probe() {
        assert!(
            Instant::now() < deadline,
            "engine/worker did not become ready"
        );
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "requires an existing native iii engine binary via III_BINARY; no WhatsApp account needed"]
fn real_engine_registers_all_schemas_and_serves_offline_session_lifecycle() {
    let binary = std::path::PathBuf::from(
        std::env::var("III_BINARY").expect("set III_BINARY to the built engine"),
    );
    let temp = TempDir::new().unwrap();
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let configuration = temp.path().join("engine.yaml");
    fs::write(&configuration, format!("workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: ./configuration\n")).unwrap();
    let engine_log = File::create(temp.path().join("engine.log")).unwrap();
    let _engine = Process(
        Command::new(&binary)
            .arg("--config")
            .arg(&configuration)
            .arg("--no-update-check")
            .current_dir(temp.path())
            .stdout(engine_log.try_clone().unwrap())
            .stderr(engine_log)
            .spawn()
            .unwrap(),
    );
    wait_for(|| trigger(&binary, port, "engine::workers::list", json!({})).is_ok());
    let worker_log = File::create(temp.path().join("worker.log")).unwrap();
    let worker_binary =
        std::env::var("WHATSAPP_BINARY").unwrap_or_else(|_| env!("CARGO_BIN_EXE_whatsapp").into());
    let mut worker = Process(
        Command::new(worker_binary)
            .args(["--url", &format!("ws://127.0.0.1:{port}")])
            .env("III_CONFIG_NAME", "whatsapp-smoke")
            .current_dir(temp.path())
            .stdout(worker_log.try_clone().unwrap())
            .stderr(worker_log)
            .spawn()
            .unwrap(),
    );
    wait_for(|| trigger(&binary, port, "whatsapp::sessions::list", json!({})).is_ok());
    assert_eq!(
        trigger(&binary, port, "whatsapp::sessions::list", json!({})).unwrap(),
        json!([])
    );
    let functions = whatsapp::registry().catalog();
    for function in &functions {
        let detail = trigger(
            &binary,
            port,
            "engine::functions::info",
            json!({"function_id": function.id}),
        )
        .unwrap();
        assert_eq!(
            detail["request_schema"], function.request_format,
            "{} request schema",
            function.id
        );
        assert_eq!(
            detail["response_schema"], function.response_format,
            "{} response schema",
            function.id
        );
    }
    let events = trigger(
        &binary,
        port,
        "engine::triggers::info",
        json!({"id":"whatsapp::events"}),
    )
    .unwrap();
    assert!(
        events["configuration_schema"].get("properties").is_some(),
        "{events}"
    );
    let created = trigger(
        &binary,
        port,
        "whatsapp::sessions::create",
        json!({"session_id":"offline"}),
    )
    .unwrap();
    assert_eq!(created["status"], "stopped");
    assert!(
        trigger(
            &binary,
            port,
            "whatsapp::messages::send_text",
            json!({"session_id":"offline", "chat_id":"+15550000001", "text":"must not send"})
        )
        .is_err()
    );
    assert!(
        !temp
            .path()
            .join("data/whatsapp/offline/credentials.db")
            .exists()
    );
    assert_eq!(
        trigger(
            &binary,
            port,
            "whatsapp::sessions::qr",
            json!({"session_id":"offline"})
        )
        .unwrap(),
        Value::Null
    );
    trigger(
        &binary,
        port,
        "whatsapp::sessions::delete",
        json!({"session_id":"offline"}),
    )
    .unwrap();
    assert!(!temp.path().join("data/whatsapp/offline").exists());
    Command::new("kill")
        .args(["-TERM", &worker.0.id().to_string()])
        .status()
        .unwrap();
    wait_for(|| worker.0.try_wait().unwrap().is_some());
    assert!(worker.0.wait().unwrap().success());
    eprintln!(
        "verified {} function schemas, event trigger, central configuration and clean SIGTERM",
        functions.len()
    );
}
