//! End-to-end test: start the engine `III_ENGINE_BIN` names on an ephemeral
//! port, run the worker binary against it, and drive the worker through
//! `iii-sdk` as a client. Skips without `III_ENGINE_BIN`; with
//! `III_E2E_REQUIRE=1` a missing engine fails the test.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use iii_sdk::protocol::CallRequest;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;
use tokio::time::{sleep, timeout};

struct Harness {
    url: String,
    engine: Child,
    worker: Child,
    dir: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.worker.kill();
        let _ = self.worker.wait();
        let _ = self.engine.kill();
        let _ = self.engine.wait();
    }
}

impl Harness {
    /// The last lines of the engine and worker logs, for a failure message.
    fn logs(&self) -> String {
        ["engine.log", "worker.log"]
            .iter()
            .map(|name| format!("--- {name}\n{}", tail(&self.dir.path().join(name), 40)))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// The engine binary, or `None` (skip) when `III_ENGINE_BIN` is unset.
fn engine_bin() -> Option<PathBuf> {
    match std::env::var_os("III_ENGINE_BIN") {
        Some(bin) => Some(bin.into()),
        None if std::env::var("III_E2E_REQUIRE").as_deref() == Ok("1") => {
            panic!("III_E2E_REQUIRE=1 but III_ENGINE_BIN is unset")
        }
        None => None,
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("bind an ephemeral port")
        .port()
}

/// stdout and stderr of a child, both appended to `dir/name`.
fn log_file(dir: &Path, name: &str) -> (Stdio, Stdio) {
    let file = std::fs::File::create(dir.join(name)).expect("create a log file");
    let err = file.try_clone().expect("clone the log file handle");
    (Stdio::from(file), Stdio::from(err))
}

/// Start the engine with only the worker manager (on an ephemeral port) and
/// the configuration store (under a temp dir) configured, then the worker
/// binary against it. The worker's configuration lives in the engine's
/// `configuration` worker; no `--config` seed is passed.
async fn boot() -> Option<Harness> {
    let bin = engine_bin()?;
    let dir = tempfile::tempdir().expect("temp dir");
    let port = free_port();
    let config = dir.path().join("config.yaml");
    std::fs::File::create(&config)
        .and_then(|mut file| {
            write!(
                file,
                "workers:\n\
                 \x20 - name: iii-worker-manager\n\
                 \x20   config:\n\
                 \x20     host: 127.0.0.1\n\
                 \x20     port: {port}\n\
                 \x20 - name: configuration\n\
                 \x20   config:\n\
                 \x20     adapter:\n\
                 \x20       name: fs\n\
                 \x20       config:\n\
                 \x20         directory: {}\n",
                dir.path().join("configuration").display()
            )
        })
        .expect("write the engine config");

    let (out, err) = log_file(dir.path(), "engine.log");
    let engine = Command::new(&bin)
        .arg("--config")
        .arg(&config)
        .arg("--no-update-check")
        .current_dir(dir.path())
        .env("III_TELEMETRY_ENABLED", "false")
        .stdout(out)
        .stderr(err)
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {}: {e}", bin.display()));

    let url = format!("ws://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(30);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            Instant::now() < deadline,
            "the engine did not listen on {port} within 30s\n{}",
            tail(&dir.path().join("engine.log"), 40)
        );
        sleep(Duration::from_millis(100)).await;
    }

    let (out, err) = log_file(dir.path(), "worker.log");
    let worker = Command::new(env!("CARGO_BIN_EXE_context-manager"))
        .args(["--url", &url])
        .current_dir(dir.path())
        .stdout(out)
        .stderr(err)
        .spawn()
        .expect("spawn the context-manager binary");

    Some(Harness {
        url,
        engine,
        worker,
        dir,
    })
}

#[tokio::test]
async fn end_to_end_via_iii_sdk() {
    let Some(h) = boot().await else {
        eprintln!("skipping: III_ENGINE_BIN is unset");
        return;
    };

    let client = register_worker(&h.url, InitOptions::default());

    // Poll until the worker's functions are routable (registration flows
    // over its own connection; no fixed sleep).
    let deadline = Instant::now() + Duration::from_secs(30);
    let payload = json!({
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }],
                       "timestamp": 1 }],
        "model": { "id": "any-model" }
    });
    let result = loop {
        let attempt = timeout(
            Duration::from_secs(5),
            client.call(CallRequest {
                function_id: "context::count-tokens".into(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(4_000),
            }),
        )
        .await;
        let last_error = match attempt {
            Ok(Ok(v)) => break v,
            Ok(Err(e)) => e.to_string(),
            Err(_) => "timed out".to_string(),
        };
        assert!(
            Instant::now() < deadline,
            "context::count-tokens did not become routable within 30s (last error: \
             {last_error})\n{}",
            h.logs()
        );
        sleep(Duration::from_millis(250)).await;
    };

    assert_eq!(result["estimator"], "heuristic");
    assert!(result["tokens"].as_u64().is_some_and(|t| t > 0));

    client.shutdown_async().await;
}
