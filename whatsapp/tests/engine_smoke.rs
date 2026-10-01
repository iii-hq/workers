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

    // Inbox reads do not need a native client and never mark a conversation read.
    // The fresh local session has neither messages nor a WhatsApp credentials DB.
    for (function, payload, rows) in [
        (
            "whatsapp::chats::list",
            json!({"session_id": "offline"}),
            "chats",
        ),
        (
            "whatsapp::messages::list",
            json!({"session_id": "offline", "chat_id": "123@g.us"}),
            "messages",
        ),
    ] {
        let page = trigger(&binary, port, function, payload).unwrap();
        assert_eq!(page[rows], json!([]), "{function}: {page}");
        assert_eq!(page["next_cursor"], Value::Null);
        assert_eq!(
            page["coverage"]["message_limit"],
            defaults["max_inbox_messages"]
        );
        assert_eq!(page["coverage"]["retention_truncated"], false);
        assert_eq!(page["coverage"]["last_history_sync_at_ms"], Value::Null);
        assert!(page["coverage"]["capture_started_at_ms"].as_u64().unwrap() > 0);
    }
    for (function, payload, expected) in [
        (
            "whatsapp::chats::list",
            json!({"session_id": "offline", "limit": 0}),
            "limit must be 1..200",
        ),
        (
            "whatsapp::chats::list",
            json!({"session_id": "offline", "limit": 201}),
            "limit must be 1..200",
        ),
        (
            "whatsapp::messages::list",
            json!({"session_id": "offline", "chat_id": "123@g.us", "limit": 0}),
            "limit must be 1..200",
        ),
        (
            "whatsapp::messages::list",
            json!({"session_id": "offline", "chat_id": "123@g.us", "limit": 201}),
            "limit must be 1..200",
        ),
        (
            "whatsapp::chats::list",
            json!({"session_id": "offline", "query": "x".repeat(257)}),
            "query must be at most 256 bytes",
        ),
        (
            "whatsapp::chats::list",
            json!({"session_id": "offline", "cursor": "invalid!"}),
            "invalid cursor",
        ),
        (
            "whatsapp::messages::list",
            json!({"session_id": "offline", "chat_id": "123@g.us", "cursor": "x".repeat(2049)}),
            "invalid cursor",
        ),
        (
            "whatsapp::messages::list",
            json!({"session_id": "offline", "chat_id": "../outside"}),
            "invalid inbox chat_id",
        ),
        (
            "whatsapp::messages::request_history",
            json!({"session_id": "offline", "chat_id": "123@g.us", "count": 0}),
            "count must be 1..100",
        ),
        (
            "whatsapp::messages::request_history",
            json!({"session_id": "offline", "chat_id": "123@g.us", "count": 101}),
            "count must be 1..100",
        ),
        (
            "whatsapp::messages::request_history",
            json!({"session_id": "offline", "chat_id": "123@g.us"}),
            "history requires a known message in this chat",
        ),
        (
            "whatsapp::messages::request_history",
            json!({"session_id": "offline", "chat_id": "../outside"}),
            "invalid inbox chat_id",
        ),
    ] {
        let error = trigger(&binary, port, function, payload).unwrap_err();
        assert!(error.contains(expected), "{function}: {error}");
    }

    // Seed only this test's isolated display archive. A short-lived second
    // SQLite connection closes before RPC reads and configuration-driven pruning.
    {
        let inbox = whatsapp::inbox::Inbox::open(
            &temp.path().join("data/whatsapp/offline"),
            whatsapp::config::Config::default().max_inbox_messages,
        )
        .unwrap();
        let fixtures = (0..4)
            .map(|number| {
                let chat = if number == 3 {
                    "15550000001@s.whatsapp.net"
                } else {
                    "123@g.us"
                };
                json!({"kind": "message.received", "data": {
                    "info": {"id": format!("fixture-{number}"), "timestamp_ms": (number + 1) * 1000,
                        "push_name": "Smoke fixture", "source": {"chat": chat,
                            "sender": "15550000001@s.whatsapp.net", "is_from_me": false}},
                    "message": {"conversation": format!("offline fixture {number}")}
                }})
            })
            .collect::<Vec<_>>();
        assert_eq!(
            inbox
                .import_event_page(&json!({"events": fixtures}))
                .unwrap(),
            4
        );
    }
    let first_chats = trigger(
        &binary,
        port,
        "whatsapp::chats::list",
        json!({"session_id": "offline", "limit": 1}),
    )
    .unwrap();
    assert_eq!(first_chats["chats"].as_array().unwrap().len(), 1);
    assert_eq!(
        first_chats["chats"][0]["chat_id"],
        "15550000001@s.whatsapp.net"
    );
    assert!(first_chats["next_cursor"].is_string());
    let second_chats = trigger(
        &binary,
        port,
        "whatsapp::chats::list",
        json!({"session_id": "offline", "limit": 1, "cursor": first_chats["next_cursor"]}),
    )
    .unwrap();
    assert_eq!(second_chats["chats"][0]["chat_id"], "123@g.us");
    assert_eq!(second_chats["chats"][0]["message_count"], 3);
    assert_eq!(second_chats["next_cursor"], Value::Null);
    let first_messages = trigger(
        &binary,
        port,
        "whatsapp::messages::list",
        json!({"session_id": "offline", "chat_id": "123@g.us", "limit": 2}),
    )
    .unwrap();
    assert_eq!(first_messages["messages"].as_array().unwrap().len(), 2);
    assert_eq!(first_messages["messages"][0]["message_id"], "fixture-2");
    assert_eq!(first_messages["messages"][0]["text"], "offline fixture 2");
    assert!(first_messages["next_cursor"].is_string());
    let second_messages = trigger(
        &binary,
        port,
        "whatsapp::messages::list",
        json!({"session_id": "offline", "chat_id": "123@g.us", "limit": 2, "cursor": first_messages["next_cursor"]}),
    )
    .unwrap();
    assert_eq!(second_messages["messages"].as_array().unwrap().len(), 1);
    assert_eq!(second_messages["messages"][0]["message_id"], "fixture-0");
    assert_eq!(second_messages["next_cursor"], Value::Null);
    for (function, payload) in [
        (
            "whatsapp::chats::list",
            json!({"session_id": "offline", "query": "fixture", "cursor": first_chats["next_cursor"]}),
        ),
        (
            "whatsapp::messages::list",
            json!({"session_id": "offline", "chat_id": "15550000001@s.whatsapp.net", "cursor": first_messages["next_cursor"]}),
        ),
    ] {
        let error = trigger(&binary, port, function, payload).unwrap_err();
        assert!(
            error.contains("cursor belongs to another chat or search"),
            "{error}"
        );
    }
    // A known anchor still cannot start history fetching for an offline session.
    let history_error = trigger(
        &binary,
        port,
        "whatsapp::messages::request_history",
        json!({"session_id": "offline", "chat_id": "123@g.us"}),
    )
    .unwrap_err();
    assert!(
        history_error.contains("session is not connected"),
        "{history_error}"
    );

    // Configuration lives in default while its subscriber and status RPC live
    // in the worker's namespace. A set must reach that subscriber by itself.
    let mut tuned = defaults.clone();
    tuned["max_sessions"] = json!(1);
    tuned["max_media_bytes"] = json!(4096);
    tuned["max_inbox_messages"] = json!(2);
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
    let retained = trigger(
        &binary,
        port,
        "whatsapp::messages::list",
        json!({"session_id": "offline", "chat_id": "123@g.us"}),
    )
    .unwrap();
    assert_eq!(retained["messages"].as_array().unwrap().len(), 1);
    assert_eq!(retained["messages"][0]["message_id"], "fixture-2");
    assert_eq!(retained["coverage"]["message_limit"], 2);
    assert_eq!(retained["coverage"]["retention_truncated"], true);
    assert_eq!(retained["next_cursor"], Value::Null);
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

    for (field, value) in [
        ("max_sessions", 0),
        ("max_inbox_messages", 0),
        ("max_inbox_messages", 100_001),
    ] {
        let mut invalid = tuned.clone();
        invalid[field] = json!(value);
        let schema_error = trigger(
            &binary,
            port,
            "configuration::set",
            json!({"id": CONFIG_ID, "value": invalid}),
        )
        .unwrap_err();
        assert!(
            schema_error.contains("SCHEMA_INVALID"),
            "{field}: {schema_error}"
        );
    }
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
        "verified {} function schemas, ADE assets, namespaced configuration reloads, offline inbox pagination and live retention, offline lifecycle and clean SIGTERM",
        functions.len()
    );
}
