//! Runtime regression test for the local/control URL selected from `III_URL`.
//! The test starts an isolated engine on a non-default port, starts the bridge
//! without `--url`, and verifies that the engine sees worker name `bridge`.

use std::io::Write as _;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};

struct Engine {
    child: Child,
    url: String,
    directory: PathBuf,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn engine_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("III_ENGINE_BIN") {
        return Some(path.into());
    }

    Command::new("iii")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .filter(|status| status.success())
        .map(|_| PathBuf::from("iii"))
}

fn free_non_default_port() -> u16 {
    loop {
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("bind an ephemeral port")
            .local_addr()
            .expect("read the ephemeral port")
            .port();
        if port != 49134 {
            return port;
        }
    }
}

async fn trigger_workers_list(client: &IIIClient) -> Option<Value> {
    client
        .trigger(TriggerRequest {
            function_id: "engine::workers::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(1_000),
        })
        .await
        .ok()
}

async fn spawn_engine() -> Option<Engine> {
    let binary = engine_binary()?;
    let port = free_non_default_port();
    let directory =
        std::env::temp_dir().join(format!("bridge-local-url-{}-{port}", std::process::id()));
    std::fs::create_dir_all(directory.join("configuration"))
        .expect("create integration test directory");

    let config = format!(
        r#"workers:
  - name: iii-worker-manager
    config:
      host: 127.0.0.1
      port: {port}
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: {directory}/configuration
      ttl_seconds: 0
"#,
        directory = directory.display()
    );
    let config_path = directory.join("config.yaml");
    std::fs::File::create(&config_path)
        .and_then(|mut file| file.write_all(config.as_bytes()))
        .expect("write engine config");

    let child = Command::new(binary)
        .args(["--no-update-check", "--config"])
        .arg(&config_path)
        .current_dir(&directory)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn iii engine");
    let engine = Engine {
        child,
        url: format!("ws://127.0.0.1:{port}"),
        directory,
    };

    let probe = register_worker(&engine.url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if trigger_workers_list(&probe).await.is_some() {
            probe.shutdown_async().await;
            return Some(engine);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    probe.shutdown_async().await;

    panic!("engine did not become ready at {}", engine.url);
}

fn bridge_is_registered(workers: &Value) -> bool {
    workers
        .get("workers")
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.get("name").and_then(Value::as_str) == Some("bridge"))
        })
}

#[tokio::test]
async fn bridge_registers_on_non_default_local_engine_from_iii_url() {
    let Some(engine) = spawn_engine().await else {
        eprintln!("skipping: no iii engine (set III_ENGINE_BIN or put `iii` on PATH)");
        return;
    };

    let remote_port = free_non_default_port();
    let seed_path = engine.directory.join("bridge-seed.yaml");
    std::fs::write(&seed_path, format!("url: ws://127.0.0.1:{remote_port}\n"))
        .expect("write bridge seed config");
    let mut worker = Worker(
        Command::new(env!("CARGO_BIN_EXE_bridge"))
            .args(["--config"])
            .arg(&seed_path)
            .env("III_URL", &engine.url)
            .env("III_CONFIG_NAME", format!("bridge-local-url-{remote_port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn bridge worker"),
    );

    let probe = register_worker(&engine.url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut last_workers = Value::Null;
    while Instant::now() < deadline {
        if let Some(status) = worker.0.try_wait().expect("query bridge process") {
            panic!("bridge exited before registration with status {status}");
        }
        if let Some(workers) = trigger_workers_list(&probe).await {
            if bridge_is_registered(&workers) {
                probe.shutdown_async().await;
                return;
            }
            last_workers = workers;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    probe.shutdown_async().await;

    panic!(
        "bridge did not register on the III_URL engine {}; last worker list: {last_workers}",
        engine.url
    );
}
