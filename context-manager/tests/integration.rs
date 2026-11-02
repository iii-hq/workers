//! End-to-end test: spawn a private `iii` engine + the worker binary as
//! subprocesses, then drive the worker through `iii-sdk` as a client.
//!
//! The engine listens on a free port and both processes run with a throwaway
//! `HOME` and working directory, so a developer's own engine (and `~/.iii`) is
//! never touched. Self-skips, printing why, when `iii` is not on PATH or the
//! engine fails to come up.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;
use tokio::time::{sleep, timeout};

/// Kills and reaps the child on drop, so every early return or panic
/// cleans up instead of orphaning the process.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Harness {
    // Fields drop in order: the worker, then its engine, then the tempdir.
    _worker: Reaped,
    _iii: Reaped,
    _dir: tempfile::TempDir,
    engine_ws: String,
}

/// Spawn `command` with a private `HOME` and working directory.
fn spawn_isolated(mut command: Command, dir: &Path) -> Option<Reaped> {
    command
        .current_dir(dir)
        .env("HOME", dir.join("home"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(Reaped)
        .inspect_err(|error| eprintln!("skipping: spawn failed: {error}"))
        .ok()
}

async fn wait_for_listen(port: u16, max: Duration) -> bool {
    let deadline = std::time::Instant::now() + max;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        sleep(Duration::from_millis(100)).await;
    }
    false
}

async fn boot() -> Option<Harness> {
    let iii_bin = which::which("iii").ok()?;
    let engine_port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("free port")
        .port();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("home")).expect("private HOME");
    let engine_config = dir.path().join("engine.yaml");
    std::fs::write(
        &engine_config,
        format!(
            r#"workers:
  - name: iii-worker-manager
    config:
      host: 127.0.0.1
      port: {engine_port}
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: "{}"
      ttl_seconds: 0
"#,
            dir.path().join("configuration").display()
        ),
    )
    .expect("write engine config");

    let mut command = Command::new(&iii_bin);
    command
        .args(["--no-update-check", "--config"])
        .arg(&engine_config);
    let iii = spawn_isolated(command, dir.path())?;
    // The configuration functions register before the engine starts listening.
    if !wait_for_listen(engine_port, Duration::from_secs(30)).await {
        eprintln!("skipping: the iii engine never listened on {engine_port}");
        return None;
    }

    // Path B: the authoritative config lives in the engine's `configuration`
    // worker; no `--config` seed is passed.
    let engine_ws = format!("ws://127.0.0.1:{engine_port}");
    let mut command = Command::new(env!("CARGO_BIN_EXE_context-manager"));
    command.args(["--url", &engine_ws]);
    let worker = spawn_isolated(command, dir.path())?;

    Some(Harness {
        _worker: worker,
        _iii: iii,
        _dir: dir,
        engine_ws,
    })
}

/// Runs the end-to-end body in a child with a private `HOME`: its SDK client
/// would otherwise write an `iii ps` marker under the real `~/.iii/run`.
#[test]
fn end_to_end_via_iii_sdk() {
    let home = tempfile::tempdir().expect("tempdir");
    let output = Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", "end_to_end_child", "--ignored", "--nocapture"])
        .env("HOME", home.path())
        .env("CONTEXT_MANAGER_E2E_CHILD", "1")
        .output()
        .expect("run the end-to-end child");
    let (stdout, stderr) = (
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    // A filter that matches nothing also exits 0.
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}\n{stderr}"
    );
    eprint!("{stderr}");
}

#[tokio::test]
#[ignore = "isolated subprocess invoked by end_to_end_via_iii_sdk"]
async fn end_to_end_child() {
    // `--ignored` runs would otherwise use the real HOME; only the parent sets this.
    if std::env::var_os("CONTEXT_MANAGER_E2E_CHILD").is_none() {
        eprintln!("skipping: runs only as the isolated child of end_to_end_via_iii_sdk");
        return;
    }
    let Some(h) = boot().await else {
        eprintln!("skipping: prerequisites not met (see logs above)");
        return;
    };

    let client = register_worker(&h.engine_ws, InitOptions::default());

    // Poll until the worker's functions are routable (registration
    // flows over its own connection; no fixed sleep).
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let payload = json!({
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }],
                       "timestamp": 1 }],
        "model": { "id": "any-model" }
    });
    let result = loop {
        let attempt = timeout(
            Duration::from_secs(5),
            client.trigger(TriggerRequest {
                function_id: "context::count-tokens".into(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(4_000),
            }),
        )
        .await;
        match attempt {
            Ok(Ok(v)) => break v,
            Ok(Err(Error::NotConnected)) | Err(_) => {}
            Ok(Err(_)) => {}
        }
        assert!(
            std::time::Instant::now() < deadline,
            "context::count-tokens did not become routable within 20s"
        );
        sleep(Duration::from_millis(250)).await;
    };

    assert_eq!(result["estimator"], "heuristic");
    assert!(result["tokens"].as_u64().is_some_and(|t| t > 0));

    client.shutdown_async().await;
}
