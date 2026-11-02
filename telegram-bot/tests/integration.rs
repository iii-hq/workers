//! End-to-end test: spawn a private `iii` engine + worker on a free port, each
//! with a throwaway `HOME`, so a developer's own engine (and `~/.iii`) is never
//! touched. Self-skips when `iii` is absent or the engine fails to come up.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

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

/// A scratch directory removed on drop; this crate has no `tempfile` dependency.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("telegram-bot-it-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Harness {
    // Fields drop in order: the worker, then its engine, then the scratch dir.
    _worker: Reaped,
    _iii: Reaped,
    _dir: Scratch,
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
    let dir = Scratch::new("engine");
    std::fs::create_dir_all(dir.0.join("home")).expect("private HOME");
    let engine_config = dir.0.join("engine.yaml");
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
            dir.0.join("configuration").display()
        ),
    )
    .expect("write engine config");

    let mut command = Command::new(&iii_bin);
    command
        .args(["--no-update-check", "--config"])
        .arg(&engine_config);
    let iii = spawn_isolated(command, &dir.0)?;
    // The configuration functions register before the engine starts listening.
    if !wait_for_listen(engine_port, Duration::from_secs(30)).await {
        eprintln!("skipping: the iii engine never listened on {engine_port}");
        return None;
    }

    let engine_ws = format!("ws://127.0.0.1:{engine_port}");
    let config = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/config.yaml");
    let mut command = Command::new(env!("CARGO_BIN_EXE_telegram-bot"));
    command.args(["--url", &engine_ws, "--config", config]);
    let worker = spawn_isolated(command, &dir.0)?;

    sleep(Duration::from_millis(1500)).await;
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
fn worker_registers_on_engine() {
    let home = Scratch::new("home");
    let output = Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", "end_to_end_child", "--ignored", "--nocapture"])
        .env("HOME", &home.0)
        .env("TELEGRAM_BOT_E2E_CHILD", "1")
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
#[ignore = "isolated subprocess invoked by worker_registers_on_engine"]
async fn end_to_end_child() {
    // `--ignored` runs would otherwise use the real HOME; only the parent sets this.
    if std::env::var_os("TELEGRAM_BOT_E2E_CHILD").is_none() {
        eprintln!("skipping: runs only as the isolated child of worker_registers_on_engine");
        return;
    }
    let Some(h) = boot().await else {
        eprintln!("skipping: prerequisites not met (see logs above)");
        return;
    };

    let client = register_worker(&h.engine_ws, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    let result = timeout(
        Duration::from_secs(10),
        client.trigger(TriggerRequest {
            function_id: "engine::functions::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5000),
        }),
    )
    .await
    .expect("trigger timed out")
    .expect("trigger failed");

    let functions = result
        .get("functions")
        .and_then(|v| v.as_array())
        .expect("functions list");
    // The engine identifies functions by `function_id`; older builds used `id`.
    // Accept either so the probe is robust to engine/SDK version skew. The
    // `telegram-bot::webhook` function is registered regardless of the active
    // updates adapter (its HTTP route is what is adapter-gated, not the
    // function itself).
    let registered = functions.iter().any(|f| {
        f.get("function_id")
            .or_else(|| f.get("id"))
            .and_then(|v| v.as_str())
            == Some("telegram-bot::webhook")
    });
    if !registered {
        panic!("telegram-bot::webhook was not registered on engine");
    }

    client.shutdown_async().await;
}
