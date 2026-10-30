//! End-to-end test: spawn an engine-only `iii` on an ephemeral port, start the
//! worker binary against it and check its functions register.
//!
//! The engine binary is `III_ENGINE_BIN`, or `iii` on PATH. Without either the
//! test skips; `III_E2E_REQUIRE=1` makes a missing or unreachable engine a
//! failure.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use iii_sdk::{register_worker, CallRequest, IIIClient, InitOptions};
use serde_json::{json, Value};
use tokio::time::sleep;

const WEBHOOK_FN: &str = "telegram-bot::webhook";

struct Engine {
    url: String,
    child: Child,
    dir: PathBuf,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn e2e_required() -> bool {
    std::env::var("III_E2E_REQUIRE").is_ok_and(|v| v == "1")
}

fn engine_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("III_ENGINE_BIN") {
        return Some(path.into());
    }
    which::which("iii").ok()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

async fn call(client: &IIIClient, function_id: &str, payload: Value) -> Option<Value> {
    client
        .call(CallRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(1000),
        })
        .await
        .ok()
}

/// An engine-only config: the worker manager on `port` and the configuration
/// store under `dir`. The worker reads its configuration from that store.
async fn spawn_engine() -> Result<Engine, String> {
    let bin = engine_bin().ok_or("no engine binary (set III_ENGINE_BIN or put `iii` on PATH)")?;
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("telegram-bot-it-{}-{port}", std::process::id()));
    std::fs::create_dir_all(dir.join("config")).map_err(|e| format!("create {dir:?}: {e}"))?;
    let config = format!(
        "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: \"{}\"\n",
        dir.join("config").display(),
    );
    let config_path = dir.join("config.yaml");
    std::fs::File::create(&config_path)
        .and_then(|mut f| f.write_all(config.as_bytes()))
        .map_err(|e| format!("write {config_path:?}: {e}"))?;

    let child = Command::new(&bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config_path)
        .current_dir(&dir)
        .env("III_TELEMETRY_ENABLED", "false")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {bin:?}: {e}"))?;
    let engine = Engine {
        url: format!("ws://127.0.0.1:{port}"),
        child,
        dir,
    };

    // Ready once the engine answers and the configuration worker serves: a
    // not-found for an unknown id proves it is up.
    let probe = register_worker(&engine.url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let core = call(&probe, "engine::workers::list", json!({})).await.is_some();
        let configuration = core
            && match probe
                .call(CallRequest {
                    function_id: "configuration::get".into(),
                    payload: json!({ "id": "__readiness_probe__" }),
                    action: None,
                    timeout_ms: Some(1000),
                })
                .await
            {
                Ok(_) => true,
                Err(e) => e.to_string().to_ascii_uppercase().contains("NOT_FOUND"),
            };
        if configuration {
            break;
        }
        if Instant::now() > deadline {
            probe.shutdown_async().await;
            return Err(format!("engine {bin:?} did not come up on {}", engine.url));
        }
        sleep(Duration::from_millis(250)).await;
    }
    probe.shutdown_async().await;
    Ok(engine)
}

fn spawn_worker(url: &str) -> Worker {
    let config = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/config.yaml");
    let child = Command::new(env!("CARGO_BIN_EXE_telegram-bot"))
        .arg("--url")
        .arg(url)
        .arg("--config")
        .arg(config)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn telegram-bot");
    Worker(child)
}

fn lists_function(list: &Value, id: &str) -> bool {
    list.get("functions")
        .and_then(Value::as_array)
        .is_some_and(|functions| {
            functions
                .iter()
                .any(|f| f.get("function_id").and_then(Value::as_str) == Some(id))
        })
}

#[tokio::test]
async fn worker_registers_on_engine() {
    let engine = match spawn_engine().await {
        Ok(engine) => engine,
        Err(reason) if !e2e_required() => {
            eprintln!("skipping: {reason}");
            return;
        }
        Err(reason) => panic!("{reason}"),
    };
    let _worker = spawn_worker(&engine.url);

    // `telegram-bot::webhook` registers whatever the updates adapter; only its
    // `http::on-request` observer depends on the adapter.
    let client = register_worker(&engine.url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(30);
    let registered = loop {
        if let Some(list) = call(&client, "engine::functions::list", json!({})).await {
            if lists_function(&list, WEBHOOK_FN) {
                break true;
            }
        }
        if Instant::now() > deadline {
            break false;
        }
        sleep(Duration::from_millis(250)).await;
    };
    client.shutdown_async().await;
    assert!(registered, "{WEBHOOK_FN} was not registered on the engine");
}
