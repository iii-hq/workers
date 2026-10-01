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

const NAMESPACE: &str = "whatsapp-smoke";
const CONFIG_ID: &str = "whatsapp-smoke";

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn trigger(binary: &Path, port: u16, id: &str, payload: Value) -> Result<Value, String> {
    let namespace = if id.starts_with("whatsapp::") {
        NAMESPACE
    } else {
        "default"
    };
    let output = Command::new(binary)
        .args([
            "trigger",
            "--engine",
            &format!("ws://127.0.0.1:{port}"),
            "--namespace",
            namespace,
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
            .env_remove("WHATSAPP_SMOKE_UNSET")
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
            .env("III_CONFIG_NAME", CONFIG_ID)
            .env("III_NAMESPACE", NAMESPACE)
            .env_remove("III_WHATSAPP_UI_WATCH")
            .current_dir(temp.path())
            .stdout(worker_log.try_clone().unwrap())
            .stderr(worker_log)
            .spawn()
            .unwrap(),
    );
    wait_for(|| trigger(&binary, port, "whatsapp::sessions::list", json!({})).is_ok());
    let defaults = serde_json::to_value(whatsapp::config::Config::default()).unwrap();
    wait_for(|| {
        trigger(&binary, port, "whatsapp::config-status", json!({})).is_ok_and(|status| {
            status["id"] == CONFIG_ID
                && status["source"] == "central"
                && status["last_outcome"] == "applied"
                && status["active"] == defaults
        })
    });
    assert_eq!(
        trigger(&binary, port, "whatsapp::configuration-id", json!({})).unwrap(),
        json!({"id": CONFIG_ID})
    );
    let schema = trigger(
        &binary,
        port,
        "configuration::schema",
        json!({"id": CONFIG_ID}),
    )
    .unwrap();
    assert_eq!(schema["metadata"]["ui_form"], "whatsapp");
    assert!(schema["schema"]["properties"].get("data_dir").is_some());
    for (path, content_type) in [
        ("whatsapp/page.js", "text/javascript"),
        ("whatsapp/styles.css", "text/css"),
    ] {
        wait_for(|| trigger(&binary, port, "whatsapp::ui-content", json!({"path": path})).is_ok());
        let asset = trigger(&binary, port, "whatsapp::ui-content", json!({"path": path})).unwrap();
        assert!(
            asset["content"]
                .as_str()
                .is_some_and(|text| !text.trim().is_empty())
        );
        assert!(
            asset["content_type"]
                .as_str()
                .is_some_and(|value| value.starts_with(content_type)),
            "{path}: {asset}"
        );
    }
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
            json!({"function_id": function.id, "namespace": NAMESPACE}),
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
        json!({"id":"whatsapp::events", "namespace": NAMESPACE}),
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

    // Configuration lives in default while its subscriber and status RPC live
    // in the worker's namespace. A set must reach that subscriber by itself.
    let mut tuned = defaults.clone();
    tuned["max_sessions"] = json!(1);
    tuned["max_media_bytes"] = json!(4096);
    tuned["operation_timeout_secs"] = json!(9);
    tuned["shutdown_timeout_secs"] = json!(3);
    trigger(
        &binary,
        port,
        "configuration::set",
        json!({"id": CONFIG_ID, "value": tuned}),
    )
    .unwrap();
    wait_for(|| {
        trigger(&binary, port, "whatsapp::config-status", json!({}))
            .is_ok_and(|status| status["last_outcome"] == "applied" && status["active"] == tuned)
    });
    let limit_error = trigger(
        &binary,
        port,
        "whatsapp::sessions::create",
        json!({"session_id":"over-limit"}),
    )
    .unwrap_err();
    assert!(
        limit_error.contains("maximum session count"),
        "{limit_error}"
    );
    assert_eq!(
        trigger(&binary, port, "whatsapp::sessions::list", json!({})).unwrap(),
        json!([created])
    );

    // Callback payloads are only hints: even a direct invocation must fetch
    // the stored value instead of accepting an injected configuration body.
    assert_eq!(
        trigger(
            &binary,
            port,
            "whatsapp::on-config-change",
            json!({"id": CONFIG_ID, "value": {"max_sessions": 999}}),
        )
        .unwrap(),
        json!({"ok": true})
    );
    let status = trigger(&binary, port, "whatsapp::config-status", json!({})).unwrap();
    assert_eq!(status["active"], tuned);
    assert_eq!(status["last_error"], Value::Null);
    assert_eq!(status["rejected_reloads"], 0);
    assert_eq!(status["restart_required_fields"], json!([]));

    let mut invalid = tuned.clone();
    invalid["max_sessions"] = json!(0);
    let schema_error = trigger(
        &binary,
        port,
        "configuration::set",
        json!({"id": CONFIG_ID, "value": invalid}),
    )
    .unwrap_err();
    assert!(schema_error.contains("SCHEMA_INVALID"), "{schema_error}");
    assert_eq!(
        trigger(
            &binary,
            port,
            "configuration::get",
            json!({"id": CONFIG_ID})
        )
        .unwrap()["value"],
        tuned
    );

    // The central store accepts unresolved environment templates, but a
    // consumer cannot load one. This exercises last-good retention through a
    // real configuration event without weakening the registered schema.
    let mut unresolved = tuned.clone();
    unresolved["data_dir"] = json!("${WHATSAPP_SMOKE_UNSET}");
    trigger(
        &binary,
        port,
        "configuration::set",
        json!({"id": CONFIG_ID, "value": unresolved}),
    )
    .unwrap();
    wait_for(|| {
        trigger(&binary, port, "whatsapp::config-status", json!({})).is_ok_and(|status| {
            status["last_outcome"] == "rejected"
                && status["rejected_reloads"]
                    .as_u64()
                    .is_some_and(|count| count > 0)
        })
    });
    let rejected = trigger(&binary, port, "whatsapp::config-status", json!({})).unwrap();
    assert_eq!(rejected["active"], tuned);
    assert!(
        rejected["last_error"]
            .as_str()
            .is_some_and(|error| error.contains("EXPAND_FAILED"))
    );
    trigger(
        &binary,
        port,
        "configuration::set",
        json!({"id": CONFIG_ID, "value": tuned}),
    )
    .unwrap();
    wait_for(|| {
        trigger(&binary, port, "whatsapp::config-status", json!({})).is_ok_and(|status| {
            status["last_outcome"] == "applied"
                && status["active"] == tuned
                && status["last_error"].is_null()
        })
    });

    let mut structural = tuned.clone();
    structural["data_dir"] = json!("data/restart-only");
    structural["max_events"] = json!(17);
    structural["max_sessions"] = json!(2);
    let mut active = tuned.clone();
    active["max_sessions"] = json!(2);
    trigger(
        &binary,
        port,
        "configuration::set",
        json!({"id": CONFIG_ID, "value": structural}),
    )
    .unwrap();
    wait_for(|| {
        trigger(&binary, port, "whatsapp::config-status", json!({})).is_ok_and(|status| {
            status["last_outcome"] == "restart_required" && status["active"] == active
        })
    });
    let status = trigger(&binary, port, "whatsapp::config-status", json!({})).unwrap();
    assert_eq!(
        status["restart_required_fields"],
        json!(["data_dir", "max_events"])
    );
    assert_eq!(status["last_error"], Value::Null);
    assert!(
        status["rejected_reloads"].as_u64().unwrap()
            >= rejected["rejected_reloads"].as_u64().unwrap()
    );
    trigger(
        &binary,
        port,
        "whatsapp::sessions::create",
        json!({"session_id":"after-reload"}),
    )
    .unwrap();
    assert!(temp.path().join("data/whatsapp/offline").is_dir());
    assert!(temp.path().join("data/whatsapp/after-reload").is_dir());
    assert!(!temp.path().join("data/restart-only").exists());
    trigger(
        &binary,
        port,
        "whatsapp::sessions::delete",
        json!({"session_id":"after-reload"}),
    )
    .unwrap();
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
        "verified {} function schemas, ADE assets, namespaced configuration reloads, offline lifecycle and clean SIGTERM",
        functions.len()
    );
}
